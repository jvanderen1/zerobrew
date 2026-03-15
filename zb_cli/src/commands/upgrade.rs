use console::style;
use std::time::Instant;

use super::install::{create_progress_callback, finish_progress_bars};
use super::uninstall::uninstall_batch;
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

    // 6. Perform upgrades: uninstall old, then install new
    ui.blank_line().map_err(ui_error)?;
    let names_to_install: Vec<String> = outdated.iter().map(|p| p.name.clone()).collect();

    // Uninstall outdated packages using shared batch helper
    ui.heading("Removing outdated versions...")
        .map_err(ui_error)?;
    let errors = uninstall_batch(installer, &names_to_install, ui)?;
    for (name, err) in &errors {
        ui.warn(format!("Failed to uninstall {}: {}", name, err))
            .map_err(ui_error)?;
    }

    // Install new versions with shared progress UI
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

    Ok(())
}

fn ui_error(err: std::io::Error) -> zb_core::Error {
    zb_core::Error::FileError {
        message: format!("failed to write CLI output: {err}"),
    }
}
