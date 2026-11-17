//! Protected-root policy. Hard-coded by design; adding an exception requires a
//! decision log entry. Never weaken this to make something succeed.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub String);

#[derive(Debug, Clone)]
pub struct Guard {
    /// Everything beneath these is refused.
    trees: Vec<String>,
    /// These objects themselves are refused, not their contents.
    objects: Vec<String>,
    /// The only allowed locations under `trees`.
    exceptions: Vec<String>,
    users_dir: String,
    profile: String,
}

fn norm(p: &str) -> String {
    let p = p.strip_prefix(r"\\?\").unwrap_or(p).replace('/', "\\");
    p.trim_end_matches('\\').to_lowercase()
}

fn under(path: &str, root: &str) -> bool {
    path == root || (path.starts_with(root) && path.as_bytes().get(root.len()) == Some(&b'\\'))
}

impl Guard {
    pub fn new(windir: &str, profile: &str, program_files: &[&str], program_data: &str) -> Self {
        let (w, pr) = (norm(windir), norm(profile));
        let mut trees = vec![w.clone(), norm(program_data)];
        trees.extend(program_files.iter().map(|p| norm(p)));
        let users_dir = pr
            .rsplit_once('\\')
            .map(|x| x.0.to_string())
            .unwrap_or_default();
        let mut objects = vec![pr.clone(), users_dir.clone()];
        for s in [
            "appdata",
            "appdata\\local",
            "appdata\\roaming",
            "appdata\\locallow",
            "desktop",
            "documents",
            "downloads",
            "pictures",
            "music",
            "videos",
            "onedrive",
        ] {
            objects.push(format!("{pr}\\{s}"));
        }
        Self {
            trees,
            objects,
            exceptions: vec![format!("{w}\\temp")],
            users_dir,
            profile: pr,
        }
    }

    /// Builds from this process's environment. Fails closed if it is incomplete.
    pub fn from_env() -> Result<Self, Refusal> {
        let get = |k: &str| std::env::var(k).map_err(|_| Refusal(format!("{k} is not set")));
        let (w, p, pf, pd) = (
            get("WINDIR")?,
            get("USERPROFILE")?,
            get("ProgramFiles")?,
            get("ProgramData")?,
        );
        let pf86 = std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| pf.clone());
        Ok(Self::new(&w, &p, &[&pf, &pf86], &pd))
    }

    /// Ok only if a plan may touch this path.
    pub fn check(&self, path: &str) -> Result<(), Refusal> {
        let raw = path
            .strip_prefix(r"\\?\")
            .unwrap_or(path)
            .replace('/', "\\");
        let b = raw.as_bytes();
        let absolute = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\';
        if !absolute {
            return Err(Refusal("not an absolute local path".into()));
        }
        if raw.split('\\').any(|s| s == ".." || s == ".") {
            return Err(Refusal("path contains relative segments".into()));
        }
        let p = norm(&raw);
        if p.len() <= 3 {
            return Err(Refusal("volume root".into()));
        }
        let first = p[3..].split('\\').next().unwrap_or("");
        if first.starts_with('$') || first == "system volume information" {
            return Err(Refusal("volume metadata".into()));
        }
        if matches!(
            first,
            "pagefile.sys" | "hiberfil.sys" | "swapfile.sys" | "bootmgr" | "recovery" | "boot"
        ) {
            return Err(Refusal("system file".into()));
        }
        if self.objects.contains(&p) {
            return Err(Refusal("essential folder".into()));
        }
        if !self.users_dir.is_empty() && under(&p, &self.users_dir) && !under(&p, &self.profile) {
            return Err(Refusal("another user's profile".into()));
        }
        // An exception permits contents only, never the folder itself.
        if self
            .exceptions
            .iter()
            .any(|e| p.starts_with(&format!("{e}\\")))
        {
            return Ok(());
        }
        if self.trees.iter().any(|t| under(&p, t)) {
            return Err(Refusal("protected system location".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g() -> Guard {
        Guard::new(
            r"C:\Windows",
            r"C:\Users\zaber",
            &[r"C:\Program Files", r"C:\Program Files (x86)"],
            r"C:\ProgramData",
        )
    }

    #[test]
    fn refuses() {
        let g = g();
        for p in [
            r"C:\",
            "C:",
            r"C:\Windows",
            r"c:\WINDOWS\System32\x.dll",
            r"C:\Windows\Temp",
            r"C:\Windows\WinSxS\a",
            r"C:\Program Files\App",
            r"C:\Program Files (x86)\App\a.exe",
            r"C:\ProgramData\Microsoft\Windows\WER",
            r"C:\Users",
            r"C:\Users\zaber",
            r"C:\Users\zaber\Documents",
            r"C:\Users\zaber\AppData",
            r"C:\Users\zaber\AppData\Local",
            r"C:\Users\other\AppData\Local\Temp\x",
            r"C:\Users\Public\x",
            r"C:\$Recycle.Bin\x",
            r"C:\$MFT",
            r"E:\System Volume Information\a",
            r"C:\pagefile.sys",
            r"\\?\C:\Windows\x",
            r"C:\Users\zaber\Temp\..\..\..\Windows\x",
            r"relative\path",
            r"\\server\share\x",
            "",
        ] {
            assert!(g.check(p).is_err(), "should refuse {p}");
        }
    }

    #[test]
    fn allows() {
        let g = g();
        for p in [
            r"C:\Windows\Temp\old.log",
            r"C:\Users\zaber\AppData\Local\Temp\x.tmp",
            r"C:\Users\zaber\Documents\report.docx",
            r"E:\Downloads\big.iso",
            r"\\?\E:\a\b",
            r"C:\Windowsish\x",
            r"C:\Users\zaberish\x"
                .replace("zaberish", "zaber\\x")
                .as_str(),
        ] {
            assert!(g.check(p).is_ok(), "should allow {p}");
        }
    }
}
