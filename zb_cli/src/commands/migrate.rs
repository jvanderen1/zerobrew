use crate::ui::{PromptDefault, StdUi};
use console::style;
use std::process::Command;

pub async fn execute(
    installer: &mut zb_io::Installer,
    yes: bool,
    force: bool,
    ui: &mut StdUi,
) -> Result<(), zb_core::Error> {
    ui.heading("Fetching installed Homebrew packages...")
        .map_err(ui_error)?;

    let packages = match zb_io::get_homebrew_packages() {
        Ok(pkgs) => pkgs,
        Err(e) => {
            return Err(zb_core::Error::StoreCorruption {
                message: format!("Failed to get Homebrew packages: {}", e),
            });
        }
    };

    if packages.formulas.is_empty()
        && packages.non_core_formulas.is_empty()
        && packages.casks.is_empty()
    {
        ui.println("No Homebrew packages installed.")
            .map_err(ui_error)?;
        return Ok(());
    }

    ui.println(format!(
        "{} core formulas, {} non-core formulas, {} casks found",
        style(packages.formulas.len()).green(),
        style(packages.non_core_formulas.len()).yellow(),
        style(packages.casks.len()).green()
    ))
    .map_err(ui_error)?;
    ui.blank_line().map_err(ui_error)?;

    if !packages.non_core_formulas.is_empty() {
        ui.note("Formulas from non-core taps cannot be migrated to zerobrew:")
            .map_err(ui_error)?;
        for pkg in &packages.non_core_formulas {
            ui.bullet(format!("{} ({})", pkg.name, pkg.tap))
                .map_err(ui_error)?;
        }
        ui.blank_line().map_err(ui_error)?;
    }

    if !packages.casks.is_empty() {
        ui.note("Casks cannot be migrated to zerobrew (only CLI formulas are supported):")
            .map_err(ui_error)?;
        for cask in &packages.casks {
            ui.bullet(&cask.name).map_err(ui_error)?;
        }
        ui.blank_line().map_err(ui_error)?;
    }

    if packages.formulas.is_empty() {
        ui.println("No core formulas to migrate.")
            .map_err(ui_error)?;
        return Ok(());
    }

    ui.println(format!(
        "The following {} formulas will be migrated:",
        packages.formulas.len()
    ))
    .map_err(ui_error)?;
    for pkg in &packages.formulas {
        ui.bullet(&pkg.name).map_err(ui_error)?;
    }
    ui.blank_line().map_err(ui_error)?;

    if !yes
        && !ui
            .prompt_yes_no("Continue with migration? [y/N]", PromptDefault::No)
            .map_err(ui_error)?
    {
        ui.println("Aborted.").map_err(ui_error)?;
        return Ok(());
    }

    ui.blank_line().map_err(ui_error)?;
    ui.heading(format!(
        "Migrating {} formulas to zerobrew...",
        style(packages.formulas.len()).green().bold()
    ))
    .map_err(ui_error)?;

    let mut success_count = 0;
    let mut failed: Vec<String> = Vec::new();

    let formula_names: Vec<String> = packages.formulas.iter().map(|f| f.name.clone()).collect();

    match crate::commands::install::execute(
        installer,
        formula_names.clone(),
        false, // no_link
        false, // build_from_source
        ui,
    )
    .await
    {
        Ok(_) => {
            if let Ok(installed_kegs) = installer.list_installed() {
                let installed_names: std::collections::HashSet<String> =
                    installed_kegs.into_iter().map(|k| k.name).collect();
                for name in &formula_names {
                    if installed_names.contains(name) {
                        success_count += 1;
                    } else {
                        failed.push(name.clone());
                    }
                }
            } else {
                success_count = formula_names.len();
            }
        }
        Err(_) => {
            if let Ok(installed_kegs) = installer.list_installed() {
                let installed_names: std::collections::HashSet<String> =
                    installed_kegs.into_iter().map(|k| k.name).collect();
                for name in &formula_names {
                    if installed_names.contains(name) {
                        success_count += 1;
                    } else {
                        failed.push(name.clone());
                    }
                }
            } else {
                failed = formula_names.clone();
            }
        }
    }

    ui.blank_line().map_err(ui_error)?;
    ui.heading(format!(
        "Migrated {} of {} formulas to zerobrew",
        style(success_count).green().bold(),
        packages.formulas.len()
    ))
    .map_err(ui_error)?;

    if !failed.is_empty() {
        ui.note(format!("Failed to migrate {} formula(s):", failed.len()))
            .map_err(ui_error)?;
        for name in &failed {
            ui.bullet(name).map_err(ui_error)?;
        }
        ui.blank_line().map_err(ui_error)?;
    }

    if success_count == 0 {
        ui.println("No formulas were successfully migrated. Skipping uninstall from Homebrew.")
            .map_err(ui_error)?;
        return Ok(());
    }

    ui.blank_line().map_err(ui_error)?;
    if !yes
        && !ui
            .prompt_yes_no(
                &format!(
                    "Uninstall {} formula(s) from Homebrew? [y/N]",
                    style(success_count).green()
                ),
                PromptDefault::No,
            )
            .map_err(ui_error)?
    {
        ui.println("Skipped uninstall from Homebrew.")
            .map_err(ui_error)?;
        return Ok(());
    }

    ui.blank_line().map_err(ui_error)?;
    ui.heading("Uninstalling from Homebrew...")
        .map_err(ui_error)?;

    let mut uninstalled = 0;

    let mut uninstall_targets: Vec<String> = Vec::new();
    for pkg in &packages.formulas {
        if !failed.contains(&pkg.name) {
            uninstall_targets.push(pkg.name.clone());
        }
    }

    if uninstall_targets.is_empty() {
        return Ok(());
    }

    ui.step_start(format!(
        "uninstalling {} formulas combined",
        uninstall_targets.len()
    ))
    .map_err(ui_error)?;

    let mut args = vec!["uninstall"];
    if force {
        args.push("--force");
    }
    for target in &uninstall_targets {
        args.push(target);
    }

    let status = Command::new("brew")
        .args(&args)
        .status()
        .map_err(|e| format!("Failed to run brew uninstall: {}", e));

    let uninstall_failed = match status {
        Ok(s) if s.success() => {
            ui.step_ok().map_err(ui_error)?;
            uninstalled = uninstall_targets.len();
            Vec::new()
        }
        Ok(_) => {
            ui.step_fail().map_err(ui_error)?;
            uninstall_targets.clone()
        }
        Err(e) => {
            ui.step_fail().map_err(ui_error)?;
            ui.error(e).map_err(ui_error)?;
            uninstall_targets.clone()
        }
    };

    ui.blank_line().map_err(ui_error)?;
    ui.heading(format!(
        "Uninstalled {} of {} formula(s) from Homebrew",
        style(uninstalled).green().bold(),
        success_count
    ))
    .map_err(ui_error)?;

    if !uninstall_failed.is_empty() {
        ui.note(format!(
            "Failed to uninstall {} formula(s) from Homebrew:",
            uninstall_failed.len()
        ))
        .map_err(ui_error)?;
        for name in &uninstall_failed {
            ui.bullet(name).map_err(ui_error)?;
        }
        ui.println("You may need to uninstall these manually with:")
            .map_err(ui_error)?;
        ui.println("    brew uninstall --force <formula>")
            .map_err(ui_error)?;
    }

    Ok(())
}

fn ui_error(err: std::io::Error) -> zb_core::Error {
    zb_core::Error::StoreCorruption {
        message: format!("failed to write CLI output: {err}"),
    }
}
