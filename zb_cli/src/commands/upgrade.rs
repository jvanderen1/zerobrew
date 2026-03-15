use console::style;
use std::time::Instant;

use super::install::{create_progress_callback, finish_progress_bars};
use crate::ui::StdUi;
use crate::utils::normalize_formula_name;

pub async fn execute(
    installer: &mut zb_io::Installer,
    formulas: Vec<String>,
    dry_run: bool,
    build_from_source: bool,
    ui: &mut StdUi,
) -> Result<(), zb_core::Error> {
    let start = Instant::now();

    // 1. Determine which packages are outdated
    let (outdated, warnings) = if formulas.is_empty() {
        ui.heading("Checking for outdated packages...")
            .map_err(ui_error)?;
        installer.check_outdated().await?
    } else {
        let mut outdated = Vec::new();
        let mut warnings = Vec::new();
        for formula in &formulas {
            let name = normalize_formula_name(formula)?;
            match installer.is_outdated(&name).await {
                Ok(Some(pkg)) => outdated.push(pkg),
                Ok(None) => {
                    warnings.push(format!("{} is already up-to-date", name));
                }
                Err(e) => {
                    warnings.push(format!("{}: {}", name, e));
                }
            }
        }
        (outdated, warnings)
    };

    // 2. Print warnings
    for warning in &warnings {
        ui.warn(warning).map_err(ui_error)?;
    }

    // 3. Handle empty outdated list
    if outdated.is_empty() {
        ui.heading("Everything is up to date.").map_err(ui_error)?;
        return Ok(());
    }

    // 4. Print upgrade plan
    let action = if dry_run {
        "Would upgrade"
    } else {
        "Upgrading"
    };
    ui.heading(format!(
        "{} {} {}...",
        action,
        outdated.len(),
        if outdated.len() == 1 {
            "package"
        } else {
            "packages"
        }
    ))
    .map_err(ui_error)?;

    for pkg in &outdated {
        ui.bullet(format!(
            "{} {} {} {}",
            style(&pkg.name).green(),
            style(&pkg.installed_version).red(),
            style("→").dim(),
            style(&pkg.current_version).green(),
        ))
        .map_err(ui_error)?;
    }

    // 5. Dry-run: stop here
    if dry_run {
        return Ok(());
    }

    // 6. Record old versions before installing (for post-install cleanup)
    let old_versions: Vec<(String, String)> = outdated
        .iter()
        .map(|p| (p.name.clone(), p.installed_version.clone()))
        .collect();

    // 7. Install new versions first (safe: old cellar folders remain until cleanup)
    //    The DB upsert atomically updates the record and symlinks point to the new version.
    ui.blank_line().map_err(ui_error)?;
    let names_to_install: Vec<String> = outdated.iter().map(|p| p.name.clone()).collect();

    let plan = installer
        .plan_with_options(&names_to_install, build_from_source)
        .await?;

    let (bars, progress_callback) = create_progress_callback("upgraded");

    ui.heading("Downloading and installing new versions...")
        .map_err(ui_error)?;

    let result = installer
        .execute_with_progress(plan, true, Some(progress_callback))
        .await;

    finish_progress_bars(&bars);

    result?;

    // 8. Post-install cleanup: remove old cellar entries now that new versions are confirmed
    ui.heading("Cleaning up old versions...")
        .map_err(ui_error)?;
    let mut cleanup_warnings: Vec<String> = Vec::new();
    for (name, old_version) in &old_versions {
        let old_keg = installer.keg_path(name, old_version);
        if old_keg.exists() {
            if let Err(e) = std::fs::remove_dir_all(&old_keg) {
                cleanup_warnings.push(format!(
                    "Failed to remove old keg {}/{}: {}",
                    name, old_version, e
                ));
            }
        }
    }

    let upgraded_count = outdated.len();
    let elapsed = start.elapsed();
    ui.blank_line().map_err(ui_error)?;
    ui.heading(format!(
        "Upgraded {} {} in {:.2}s",
        style(upgraded_count).green().bold(),
        if upgraded_count == 1 {
            "package"
        } else {
            "packages"
        },
        elapsed.as_secs_f64()
    ))
    .map_err(ui_error)?;

    // Surface cleanup warnings after the success summary so they're visible
    for warning in &cleanup_warnings {
        ui.warn(warning).map_err(ui_error)?;
    }
    if !cleanup_warnings.is_empty() {
        ui.note("Run `zb gc` to retry cleaning up old versions.")
            .map_err(ui_error)?;
    }

    Ok(())
}

fn ui_error(err: std::io::Error) -> zb_core::Error {
    zb_core::Error::FileError {
        message: format!("failed to write CLI output: {err}"),
    }
}
