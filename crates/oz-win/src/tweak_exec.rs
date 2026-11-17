//! Shared apply / prior / revert for catalogued tweaks. The app uses it for per-user tweaks; the
//! elevated helper uses it for machine-wide ones. Both look tweaks up by id in the compiled catalog.
use crate::tweak::{self as w, RegVal};
use oz_tweaks::{Kind, Tweak};

fn parts(t: &Tweak) -> (&str, &str, &str) {
    (
        t.hive.as_deref().unwrap_or(""),
        t.key.as_deref().unwrap_or(""),
        t.value.as_deref().unwrap_or(""),
    )
}

fn target(t: &Tweak) -> Result<RegVal, String> {
    match t.kind {
        Kind::RegDword => {
            t.on.parse()
                .map(RegVal::Dword)
                .map_err(|_| "bad target".to_string())
        }
        Kind::RegSz => Ok(RegVal::Sz(t.on.clone())),
        _ => Err("not a registry tweak".into()),
    }
}

/// Encoded prior state: "none", "dword:N", "sz:TEXT", "scheme:GUID" or "power:N".
pub fn prior(t: &Tweak) -> Result<String, String> {
    Ok(match t.kind {
        Kind::RegDword | Kind::RegSz => {
            let (h, k, v) = parts(t);
            match w::reg_read(h, k, v)? {
                None => "none".into(),
                Some(RegVal::Dword(d)) => format!("dword:{d}"),
                Some(RegVal::Sz(s)) => format!("sz:{s}"),
            }
        }
        Kind::PowerPlan => format!("scheme:{}", w::active_scheme()?),
        Kind::PowerSetting => format!(
            "power:{}",
            w::power_value(
                t.subgroup.as_deref().unwrap_or(""),
                t.setting.as_deref().unwrap_or("")
            )?
        ),
    })
}

/// Applies the tweak's target. `machine` allows HKLM and power settings (helper only).
pub fn apply(t: &Tweak, machine: bool) -> Result<(), String> {
    match t.kind {
        Kind::RegDword | Kind::RegSz => {
            let (h, k, v) = parts(t);
            if machine {
                w::reg_write_machine(h, k, v, &target(t)?)
            } else {
                w::reg_write(h, k, v, &target(t)?)
            }
        }
        Kind::PowerPlan => w::set_active_scheme(&t.on),
        Kind::PowerSetting if machine => w::set_power_value(
            t.subgroup.as_deref().unwrap_or(""),
            t.setting.as_deref().unwrap_or(""),
            t.on.parse().map_err(|_| "bad target")?,
        ),
        Kind::PowerSetting => {
            Err("machine-wide settings are applied by the elevated helper".into())
        }
    }
}

/// Restores an encoded prior state. Rejects a prior that doesn't fit the tweak's kind.
pub fn revert(t: &Tweak, prior: &str, machine: bool) -> Result<(), String> {
    match t.kind {
        Kind::RegDword | Kind::RegSz => {
            let (h, k, v) = parts(t);
            let val = if prior == "none" {
                None
            } else if let Some(d) = prior.strip_prefix("dword:") {
                Some(RegVal::Dword(d.parse().map_err(|_| "bad prior")?))
            } else if let Some(s) = prior.strip_prefix("sz:") {
                Some(RegVal::Sz(s.to_string()))
            } else {
                return Err("bad prior".into());
            };
            match (val, machine) {
                (Some(x), true) => w::reg_write_machine(h, k, v, &x),
                (Some(x), false) => w::reg_write(h, k, v, &x),
                (None, true) => w::reg_delete_machine(h, k, v),
                (None, false) => w::reg_delete(h, k, v),
            }
        }
        Kind::PowerPlan => {
            let g = prior.strip_prefix("scheme:").ok_or("bad prior")?;
            w::parse_guid(g).ok_or("bad prior")?;
            w::set_active_scheme(g)
        }
        Kind::PowerSetting if machine => {
            let n: u32 = prior
                .strip_prefix("power:")
                .and_then(|x| x.parse().ok())
                .ok_or("bad prior")?;
            w::set_power_value(
                t.subgroup.as_deref().unwrap_or(""),
                t.setting.as_deref().unwrap_or(""),
                n,
            )
        }
        Kind::PowerSetting => {
            Err("machine-wide settings are applied by the elevated helper".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_malformed_priors() {
        let cat = oz_tweaks::load(oz_tweaks::CATALOG).unwrap();
        let reg = cat
            .iter()
            .find(|t| t.kind == Kind::RegDword && t.hive.as_deref() == Some("HKCU"))
            .unwrap();
        assert!(revert(reg, "dword:abc", false).is_err());
        assert!(revert(reg, "rm -rf", false).is_err());
        let plan = cat.iter().find(|t| t.kind == Kind::PowerPlan).unwrap();
        assert!(revert(plan, "scheme:not-a-guid", false).is_err());
        let ps = cat.iter().find(|t| t.kind == Kind::PowerSetting).unwrap();
        assert!(apply(ps, false).is_err(), "power settings need the helper");
    }
    #[test]
    fn priors_read_for_every_tweak() {
        for t in oz_tweaks::load(oz_tweaks::CATALOG).unwrap() {
            let _ = prior(&t);
        }
    }
}
