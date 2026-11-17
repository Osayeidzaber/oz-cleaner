//! Lists and removes Store apps for the current user through Windows' own PowerShell Appx module.
//! The command text is fixed; the only variable part is a package name that passes
//! `oz_tweaks::bloat::safe_package_name` and was read back from Get-AppxPackage.
use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub full_name: String,
    pub family: String,
    pub non_removable: bool,
    pub location: String,
}

fn powershell() -> String {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    format!(r"{windir}\System32\WindowsPowerShell\v1.0\powershell.exe")
}

fn run(script: &str) -> Result<String, String> {
    let out = Command::new(powershell())
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("couldn't start Windows PowerShell: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("Windows refused")
            .trim()
            .to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// One line per package, tab-separated, to avoid needing a JSON parser here.
pub fn list() -> Result<Vec<Package>, String> {
    let text = run("Get-AppxPackage | ForEach-Object { \"$($_.Name)`t$($_.PackageFullName)`t$($_.PackageFamilyName)`t$($_.NonRemovable)`t$($_.InstallLocation)\" }")?;
    Ok(text
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.trim_end_matches('\r').split('\t').collect();
            (f.len() == 5).then(|| Package {
                name: f[0].into(),
                full_name: f[1].into(),
                family: f[2].into(),
                non_removable: f[3].eq_ignore_ascii_case("true"),
                location: f[4].into(),
            })
        })
        .collect())
}

/// Removes one package for the current user. `full_name` must already be validated by the caller.
pub fn remove(full_name: &str) -> Result<(), String> {
    run(&format!(
        "Remove-AppxPackage -Package '{full_name}' -ErrorAction Stop"
    ))
    .map(|_| ())
}
