//! Preinstalled apps Oz may offer to remove for the current user (AGENTS.md section 6).
//! Every entry is a Microsoft Store app that can be reinstalled from the Store. Apps games or
//! Windows features depend on (Xbox identity, Store, App Installer, codecs) are not listed.

pub struct Bloat {
    /// Package family name prefix, matched against Get-AppxPackage's PackageFamilyName.
    pub family: &'static str,
    pub name: &'static str,
    pub explain: &'static str,
    pub recommended: bool,
}

pub const BLOAT: &[Bloat] = &[
    Bloat {
        family: "Microsoft.BingNews_",
        name: "Microsoft News",
        explain: "News headlines app.",
        recommended: true,
    },
    Bloat {
        family: "Microsoft.BingWeather_",
        name: "Weather",
        explain: "Weather forecasts app.",
        recommended: false,
    },
    Bloat {
        family: "Microsoft.BingSearch_",
        name: "Bing Search",
        explain: "Bing search app.",
        recommended: true,
    },
    Bloat {
        family: "Microsoft.GetHelp_",
        name: "Get Help",
        explain: "Microsoft support chat app.",
        recommended: true,
    },
    Bloat {
        family: "Microsoft.Getstarted_",
        name: "Tips",
        explain: "Windows tips and introductions.",
        recommended: true,
    },
    Bloat {
        family: "Microsoft.MicrosoftSolitaireCollection_",
        name: "Solitaire Collection",
        explain: "Card games with ads.",
        recommended: true,
    },
    Bloat {
        family: "Microsoft.People_",
        name: "People",
        explain: "Contacts app.",
        recommended: true,
    },
    Bloat {
        family: "Microsoft.PowerAutomateDesktop_",
        name: "Power Automate",
        explain: "Desktop automation tool.",
        recommended: true,
    },
    Bloat {
        family: "Microsoft.Todos_",
        name: "Microsoft To Do",
        explain: "Task list app.",
        recommended: false,
    },
    Bloat {
        family: "Microsoft.WindowsFeedbackHub_",
        name: "Feedback Hub",
        explain: "Sends feedback to Microsoft.",
        recommended: true,
    },
    Bloat {
        family: "Microsoft.WindowsMaps_",
        name: "Maps",
        explain: "Offline and online maps.",
        recommended: true,
    },
    Bloat {
        family: "Microsoft.ZuneVideo_",
        name: "Films & TV",
        explain: "Video player and store.",
        recommended: false,
    },
    Bloat {
        family: "Microsoft.MicrosoftOfficeHub_",
        name: "Microsoft 365 (Office) hub",
        explain: "Launcher that promotes Microsoft 365. Office apps themselves aren't affected.",
        recommended: true,
    },
    Bloat {
        family: "MSTeams_",
        name: "Microsoft Teams (personal)",
        explain: "Teams chat app.",
        recommended: false,
    },
    Bloat {
        family: "MicrosoftTeams_",
        name: "Microsoft Teams (older)",
        explain: "Older Teams chat app.",
        recommended: false,
    },
    Bloat {
        family: "Clipchamp.Clipchamp_",
        name: "Clipchamp",
        explain: "Video editor.",
        recommended: true,
    },
    Bloat {
        family: "Microsoft.549981C3F5F10_",
        name: "Cortana",
        explain: "Retired voice assistant.",
        recommended: true,
    },
    Bloat {
        family: "Microsoft.Copilot_",
        name: "Copilot",
        explain: "Copilot app.",
        recommended: false,
    },
    Bloat {
        family: "MicrosoftCorporationII.MicrosoftFamily_",
        name: "Microsoft Family",
        explain: "Family safety app.",
        recommended: true,
    },
    Bloat {
        family: "MicrosoftCorporationII.QuickAssist_",
        name: "Quick Assist",
        explain:
            "Remote help tool. Remove it if nobody helps you remotely; scammers often ask for it.",
        recommended: false,
    },
    Bloat {
        family: "Microsoft.WindowsSoundRecorder_",
        name: "Sound Recorder",
        explain: "Audio recorder.",
        recommended: false,
    },
    Bloat {
        family: "Microsoft.WindowsAlarms_",
        name: "Clock",
        explain: "Alarms, timers and focus sessions.",
        recommended: false,
    },
    Bloat {
        family: "Microsoft.YourPhone_",
        name: "Phone Link",
        explain: "Connects an Android phone or iPhone.",
        recommended: false,
    },
    Bloat {
        family: "Microsoft.OutlookForWindows_",
        name: "Outlook (new)",
        explain: "New Outlook mail app. Classic Outlook isn't affected.",
        recommended: false,
    },
    Bloat {
        family: "Microsoft.MicrosoftStickyNotes_",
        name: "Sticky Notes",
        explain: "Desktop notes. Your notes sync to your Microsoft account if you signed in.",
        recommended: false,
    },
    Bloat {
        family: "Microsoft.Windows.DevHome_",
        name: "Dev Home",
        explain: "Developer dashboard (retired by Microsoft).",
        recommended: true,
    },
    Bloat {
        family: "Microsoft.GamingApp_",
        name: "Xbox",
        explain: "Xbox app for PC Game Pass. Games already installed keep working.",
        recommended: false,
    },
    Bloat {
        family: "Microsoft.ZuneMusic_",
        name: "Media Player",
        explain: "Music and video player.",
        recommended: false,
    },
    Bloat {
        family: "Microsoft.MixedReality.Portal_",
        name: "Mixed Reality Portal",
        explain: "Windows Mixed Reality headset setup.",
        recommended: true,
    },
    Bloat {
        family: "Microsoft.Microsoft3DViewer_",
        name: "3D Viewer",
        explain: "3D model viewer.",
        recommended: true,
    },
    Bloat {
        family: "Microsoft.SkypeApp_",
        name: "Skype",
        explain: "Skype (retired by Microsoft in 2025).",
        recommended: true,
    },
    Bloat {
        family: "Microsoft.MSPaint_",
        name: "Paint 3D",
        explain: "Paint 3D. Classic Paint is a different app and isn't affected.",
        recommended: true,
    },
    Bloat {
        family: "Microsoft.BingFinance_",
        name: "Money",
        explain: "Finance news app.",
        recommended: true,
    },
    Bloat {
        family: "Microsoft.BingSports_",
        name: "Sports",
        explain: "Sports news app.",
        recommended: true,
    },
];

/// Package names reach PowerShell only after this check: letters, digits, `. _ ~ -` and nothing else.
pub fn safe_package_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 256
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '~' | '-'))
}

pub fn find(family: &str) -> Option<&'static Bloat> {
    BLOAT.iter().find(|b| family.starts_with(b.family))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_are_injection_safe() {
        assert!(safe_package_name(
            "Microsoft.BingNews_4.55.1.0_x64__8wekyb3d8bbwe"
        ));
        for bad in ["a;b", "a' | rm", "$(x)", "a b", "", "a`b", "a\"b"] {
            assert!(!safe_package_name(bad), "{bad}");
        }
    }
    #[test]
    fn list_is_sane() {
        for b in BLOAT {
            assert!(b.family.ends_with('_'), "{}", b.family);
            assert!(safe_package_name(b.family.trim_end_matches('_')));
        }
        for keep in [
            "Microsoft.WindowsStore_",
            "Microsoft.DesktopAppInstaller_",
            "Microsoft.XboxIdentityProvider_",
            "Microsoft.Xbox.TCUI_",
            "Microsoft.SecHealthUI_",
            "Microsoft.WindowsCalculator_",
        ] {
            assert!(find(keep).is_none(), "{keep} must never be offered");
        }
        assert!(BLOAT.len() >= 30);
    }
}
