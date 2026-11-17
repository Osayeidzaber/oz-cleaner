//! Tweak catalog loader and safety checks (D-14, AGENTS.md section 6). Pure: no Windows calls.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tier {
    Recommended,
    Optional,
    ExplainOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    RegDword,
    RegSz,
    PowerPlan,
    PowerSetting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Restart {
    None,
    Explorer,
    Signout,
    Reboot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tweak {
    pub id: String,
    pub name: String,
    pub explain: String,
    pub category: String,
    pub tier: Tier,
    pub kind: Kind,
    #[serde(default)]
    pub hive: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    /// Target: a DWORD or string for registry kinds, a scheme GUID for power plans, an AC value for power settings.
    pub on: String,
    #[serde(default)]
    pub setting: Option<String>,
    #[serde(default)]
    pub subgroup: Option<String>,
    #[serde(default)]
    pub needs_admin: bool,
    pub restart: Restart,
    pub evidence: String,
    /// Empty until the Sandbox round-trip test has passed. Empty means: show, never apply.
    pub verified: String,
}

impl Tweak {
    /// Per-user tweaks: HKCU values and the active power plan (AGENTS.md section 6).
    pub fn per_user(&self) -> bool {
        match self.kind {
            Kind::RegDword | Kind::RegSz => self.hive.as_deref() == Some("HKCU") && !self.needs_admin,
            Kind::PowerPlan => !self.needs_admin,
            Kind::PowerSetting => false,
        }
    }

    /// Per-user tweaks may be applied on the owner's say-so; machine-wide ones need a recorded Sandbox test.
    pub fn applicable(&self) -> bool {
        self.tier != Tier::ExplainOnly && (self.per_user() || !self.verified.trim().is_empty())
    }
}

#[derive(Deserialize)]
struct Catalog {
    tweak: Vec<Tweak>,
}

pub const CATALOG: &str = include_str!("../catalog.toml");

/// Registry areas no tweak may ever touch (AGENTS.md rule 3.10). Compared case-insensitively as substrings.
pub const FORBIDDEN: [&str; 12] = [
    r"windows defender",
    r"microsoft antimalware",
    r"windowsfirewall",
    r"services\sharedaccess",
    r"services\mpssvc",
    r"windowsupdate",
    r"services\wuauserv",
    r"policies\system",
    r"control\lsa",
    r"fve",
    r"smartscreen",
    r"systemrestore",
];

fn forbidden(key: &str) -> Option<&'static str> {
    let k = key.to_ascii_lowercase();
    FORBIDDEN.iter().copied().find(|f| k.contains(f))
}

fn is_guid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                *c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

pub fn load(text: &str) -> Result<Vec<Tweak>, String> {
    let c: Catalog = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut ids = std::collections::HashSet::new();
    for t in &c.tweak {
        if !ids.insert(t.id.clone()) {
            return Err(format!("duplicate tweak {}", t.id));
        }
        if t.evidence.trim().is_empty() {
            return Err(format!("{}: evidence is required", t.id));
        }
        match t.kind {
            Kind::RegDword | Kind::RegSz => {
                let (Some(h), Some(k), Some(_)) = (&t.hive, &t.key, &t.value) else {
                    return Err(format!(
                        "{}: registry tweak needs hive, key and value",
                        t.id
                    ));
                };
                match h.as_str() {
                    "HKCU" => {}
                    "HKLM" if t.needs_admin => {}
                    "HKLM" => return Err(format!("{}: HKLM needs needs_admin", t.id)),
                    _ => return Err(format!("{}: hive must be HKCU or HKLM", t.id)),
                }
                if let Some(f) = forbidden(k) {
                    return Err(format!("{}: key touches a protected area ({f})", t.id));
                }
                if k.starts_with('\\') || k.contains("..") {
                    return Err(format!("{}: malformed key", t.id));
                }
                if t.kind == Kind::RegDword && t.on.parse::<u32>().is_err() {
                    return Err(format!("{}: on must be a DWORD", t.id));
                }
            }
            Kind::PowerPlan => {
                if !is_guid(&t.on) {
                    return Err(format!("{}: on must be a scheme GUID", t.id));
                }
            }
            Kind::PowerSetting => {
                if !t.needs_admin
                    || !t.setting.as_deref().is_some_and(is_guid)
                    || !t.subgroup.as_deref().is_some_and(is_guid)
                    || t.on.parse::<u32>().is_err()
                {
                    return Err(format!(
                        "{}: power setting needs admin, GUIDs and a numeric value",
                        t.id
                    ));
                }
            }
        }
    }
    Ok(c.tweak)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_catalog_loads() {
        let t = load(CATALOG).unwrap();
        assert!(t.len() >= 75);
        // Machine-wide tweaks stay locked until a Sandbox test is recorded.
        for x in &t {
            if !x.per_user() {
                assert_eq!(x.applicable(), !x.verified.is_empty(), "{}", x.id);
                assert!(x.needs_admin || x.kind == Kind::PowerPlan, "{}", x.id);
            }
        }
        assert!(t.iter().filter(|x| x.applicable()).count() > 50);
    }

    #[test]
    fn rejects_security_and_update_keys() {
        let base = |key: &str| {
            format!("[[tweak]]\nid='a'\nname='a'\nexplain='a'\ncategory='c'\ntier='Optional'\nkind='reg_dword'\nhive='HKLM'\nkey='{key}'\nvalue='v'\non='1'\nneeds_admin=true\nrestart='none'\nevidence='x'\nverified=''\n")
        };
        for k in [
            r"SOFTWARE\Policies\Microsoft\Windows Defender",
            r"SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU",
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System",
            r"SYSTEM\CurrentControlSet\Services\SharedAccess\Parameters",
            r"SYSTEM\CurrentControlSet\Control\Lsa",
            r"SOFTWARE\Policies\Microsoft\FVE",
        ] {
            assert!(load(&base(k)).is_err(), "{k}");
        }
        assert!(load(&base(r"SYSTEM\CurrentControlSet\Control\GraphicsDrivers")).is_ok());
    }

    #[test]
    fn rejects_bad_shapes() {
        let t = "[[tweak]]\nid='a'\nname='a'\nexplain='a'\ncategory='c'\ntier='Optional'\nrestart='none'\nevidence='x'\nverified=''\n";
        assert!(
            load(&format!(
                "{t}kind='reg_dword'\nhive='HKLM'\nkey='X'\nvalue='v'\non='1'"
            ))
            .is_err(),
            "HKLM without admin"
        );
        assert!(
            load(&format!(
                "{t}kind='reg_dword'\nhive='HKCR'\nkey='X'\nvalue='v'\non='1'"
            ))
            .is_err(),
            "bad hive"
        );
        assert!(
            load(&format!(
                "{t}kind='reg_dword'\nhive='HKCU'\nkey='X'\nvalue='v'\non='yes'"
            ))
            .is_err(),
            "non-numeric"
        );
        assert!(load(&format!("{t}kind='power_plan'\non='not-a-guid'")).is_err());
        assert!(load(&format!("{t}kind='reg_dword'\nhive='HKCU'\nkey='X'\nvalue='v'\non='1'\n[[tweak]]\nid='a'\nname='a'\nexplain='a'\ncategory='c'\ntier='Optional'\nrestart='none'\nevidence='x'\nverified=''\nkind='reg_dword'\nhive='HKCU'\nkey='X'\nvalue='v'\non='1'")).is_err(), "duplicate");
    }
}
