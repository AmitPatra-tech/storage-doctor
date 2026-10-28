//! Built-in and Microsoft Store apps (Photos, Xbox, Clipchamp, Teams, OEM
//! companion apps…). These are AppX packages, not registry-installed
//! programs, so they never appear in the classic installed-programs list and
//! have no uninstaller to run. Listing works without administrator rights;
//! removing for every user (and stopping Windows from re-adding the app for
//! new accounts) needs one elevated pass.

use serde::{Deserialize, Serialize};
use std::os::windows::process::CommandExt;
use std::path::Path;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StoreApp {
    /// What the Start menu calls it, e.g. "Microsoft News".
    pub display_name: String,
    /// Package name, e.g. "Microsoft.BingNews" — what removal targets.
    pub name: String,
    pub version: String,
    /// Publisher's common name, e.g. "Microsoft Corporation".
    pub publisher: String,
    pub install_location: String,
    /// Windows marks this package as a system component it refuses to remove.
    pub non_removable: bool,
    /// What removing it breaks, for apps where that matters.
    pub caution: Option<String>,
    pub size_bytes: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPackage {
    display_name: Option<String>,
    name: String,
    version: String,
    publisher: String,
    publisher_name: Option<String>,
    install_location: Option<String>,
    non_removable: bool,
}

/// Lists Store/built-in apps that have a Start menu entry — the ones a user
/// knows as apps. Packages without one are frameworks and components.
const LIST_SCRIPT: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$start = @{}
Get-StartApps | ForEach-Object { if ($_.AppID -match '^([^!]+)!') { if (-not $start.ContainsKey($Matches[1])) { $start[$Matches[1]] = $_.Name } } }
@(Get-AppxPackage -PackageTypeFilter Main | Where-Object { -not $_.IsFramework -and $start.ContainsKey($_.PackageFamilyName) } | ForEach-Object {
  [pscustomobject]@{
    displayName     = $start[$_.PackageFamilyName]
    name            = $_.Name
    version         = [string]$_.Version
    publisher       = [string]$_.Publisher
    publisherName   = $(try { [string]([xml](Get-Content -LiteralPath (Join-Path $_.InstallLocation 'AppxManifest.xml') -Raw)).Package.Properties.PublisherDisplayName } catch { '' })
    installLocation = [string]$_.InstallLocation
    nonRemovable    = [bool]$_.NonRemovable
  }
}) | ConvertTo-Json -Compress -Depth 2
"#;

fn run_powershell(script: &str) -> Result<String, String> {
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// "CN=Microsoft Corporation, O=Microsoft Corporation, …" → "Microsoft Corporation".
fn publisher_name(raw: &str) -> String {
    raw.split(',')
        .find_map(|part| part.trim().strip_prefix("CN="))
        .map(|s| s.trim_matches('"').to_string())
        .unwrap_or_else(|| raw.to_string())
}

pub fn list() -> Result<Vec<StoreApp>, String> {
    let json = run_powershell(LIST_SCRIPT)?;
    if json.is_empty() {
        return Ok(Vec::new());
    }
    // ConvertTo-Json emits a bare object (not an array) for a single item.
    let raw: Vec<RawPackage> = if json.starts_with('[') {
        serde_json::from_str(&json).map_err(|e| e.to_string())?
    } else {
        vec![serde_json::from_str(&json).map_err(|e| e.to_string())?]
    };
    let mut apps: Vec<StoreApp> = raw
        .into_iter()
        .map(|r| {
            let location = r.install_location.unwrap_or_default();
            let size = if location.is_empty() {
                0
            } else {
                crate::cleaner::measure_dir(Path::new(&location)).size_bytes
            };
            let caution = caution_for(&r.name).map(str::to_string);
            StoreApp {
                display_name: r.display_name.unwrap_or_else(|| r.name.clone()),
                name: r.name,
                version: r.version,
                publisher: r
                    .publisher_name
                    .filter(|n| !n.trim().is_empty() && !n.starts_with("ms-resource:"))
                    .unwrap_or_else(|| publisher_name(&r.publisher)),
                caution,
                install_location: location,
                non_removable: r.non_removable,
                size_bytes: size,
            }
        })
        .collect();
    apps.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));
    apps.dedup_by(|a, b| a.name == b.name);
    Ok(apps)
}

/// Removing these has a consequence beyond the app itself.
fn caution_for(name: &str) -> Option<&'static str> {
    match name {
        "Microsoft.WindowsStore" => Some("Without the Microsoft Store you can't install or update Store apps, including the ones listed here."),
        "Microsoft.DesktopAppInstaller" => Some("Removes winget and the App Installer used to install .msix/.appx files."),
        "NVIDIACorp.NVIDIAControlPanel" => Some("Your NVIDIA graphics settings are managed here; the driver keeps working without it."),
        "AdvancedMicroDevicesInc-2.AMDRadeonSoftware" => Some("Your AMD graphics settings are managed here; the driver keeps working without it."),
        "Microsoft.SecHealthUI" => Some("This is the Windows Security dashboard."),
        _ => None,
    }
}

/// Package names are `[A-Za-z0-9.-]`; anything else is refused rather than
/// placed into a script.
pub fn valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// Elevated script: remove the package for every user, remove its
/// provisioned copy (so Windows doesn't add it back for new accounts), then
/// record exactly what is left. Lines: `error\t<msg>`, `left_users\t<n>`,
/// `left_provisioned\t<n>`, `done`.
pub fn build_remove_script(name: &str, report: &Path) -> String {
    let rf = report.to_string_lossy().replace('\'', "''");
    format!(
        r#"$sdOut = New-Object 'System.Collections.Generic.List[string]'
$n = '{name}'
try {{ Get-AppxPackage -AllUsers -Name $n | Remove-AppxPackage -AllUsers -ErrorAction Stop }} catch {{ $sdOut.Add("error`t" + $_.Exception.Message) }}
try {{ Get-AppxProvisionedPackage -Online | Where-Object {{ $_.DisplayName -eq $n }} | Remove-AppxProvisionedPackage -Online -AllUsers -ErrorAction Stop | Out-Null }} catch {{ $sdOut.Add("error`t" + $_.Exception.Message) }}
$sdOut.Add("left_users`t" + @(Get-AppxPackage -AllUsers -Name $n).Count)
$sdOut.Add("left_provisioned`t" + @(Get-AppxProvisionedPackage -Online | Where-Object {{ $_.DisplayName -eq $n }}).Count)
$sdOut.Add('done')
try {{ [System.IO.File]::WriteAllLines('{rf}', $sdOut) }} catch {{ }}
"#
    )
}

#[derive(Debug, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct StoreRemoveResult {
    /// Gone for every user account on this PC.
    pub removed: bool,
    /// Accounts it is still installed for (0 when removed).
    pub remaining_installs: u32,
    /// Windows will still add it to new accounts / after feature updates.
    pub still_provisioned: bool,
    /// Plain-language reasons for anything that did not work.
    pub notes: Vec<String>,
}

pub fn parse_remove_report(text: &str) -> StoreRemoveResult {
    let mut r = StoreRemoveResult::default();
    let mut completed = false;
    let mut users: Option<u32> = None;
    for line in text.lines() {
        let mut parts = line.splitn(2, '\t');
        let kind = parts.next().unwrap_or("").trim_start_matches('\u{feff}');
        let val = parts.next().unwrap_or("").trim();
        match kind {
            "done" => completed = true,
            "left_users" => users = val.parse().ok(),
            "left_provisioned" => r.still_provisioned = val.parse::<u32>().unwrap_or(0) > 0,
            "error" if !val.is_empty() => {
                let msg = if val.contains("0x80073CFA") {
                    "Windows marks this app as part of the system and refused to remove it.".to_string()
                } else {
                    format!("Windows reported: {val}")
                };
                if !r.notes.contains(&msg) {
                    r.notes.push(msg);
                }
            }
            _ => {}
        }
    }
    if !completed {
        r.notes.push("The administrator step stopped before finishing.".into());
    }
    r.remaining_installs = users.unwrap_or(1);
    r.removed = completed && users == Some(0);
    if r.removed && r.still_provisioned {
        r.notes.push(
            "Removed for every account, but Windows still keeps a copy to install for new \
             accounts."
                .into(),
        );
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    /// LIVE, read-only: lists the real Store/built-in apps.
    #[test]
    #[ignore]
    fn live_list() {
        let apps = list().unwrap();
        println!("COUNT={}", apps.len());
        for a in apps.iter().take(60) {
            println!("{} | {} | {} | removable={} | {} bytes", a.display_name, a.name, a.publisher, !a.non_removable, a.size_bytes);
        }
    }

    #[test]
    fn publisher_common_name_is_extracted() {
        assert_eq!(
            publisher_name("CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US"),
            "Microsoft Corporation"
        );
        assert_eq!(publisher_name("CN=\"HP Inc.\", O=HP"), "HP Inc.");
    }

    #[test]
    fn package_names_are_validated_before_use_in_a_script() {
        assert!(valid_package_name("Microsoft.BingNews"));
        assert!(valid_package_name("AdvancedMicroDevicesInc-2.AMDRadeonSoftware"));
        assert!(!valid_package_name("x'; Remove-Item C:\\ -Recurse; '"));
        assert!(!valid_package_name(""));
        assert!(!valid_package_name("a b"));
    }

    #[test]
    fn remove_report_is_classified_exactly() {
        let ok = parse_remove_report("left_users\t0\nleft_provisioned\t0\ndone\n");
        assert!(ok.removed && !ok.still_provisioned && ok.notes.is_empty());

        let prov = parse_remove_report("left_users\t0\nleft_provisioned\t1\ndone\n");
        assert!(prov.removed && prov.still_provisioned);
        assert!(prov.notes.iter().any(|n| n.contains("new accounts")));

        let sys = parse_remove_report(
            "error\tRemoval failed. 0x80073CFA, Removal failed. Please contact your software vendor.\nleft_users\t1\nleft_provisioned\t0\ndone\n",
        );
        assert!(!sys.removed);
        assert_eq!(sys.remaining_installs, 1);
        assert!(sys.notes.iter().any(|n| n.contains("part of the system")));

        let crashed = parse_remove_report("");
        assert!(!crashed.removed);
        assert!(crashed.notes.iter().any(|n| n.contains("stopped before finishing")));
    }
}
