use console::style;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use zb_io::{InstallProgress, ProgressCallback};

use crate::ui::StdUi;
use crate::utils::{normalize_formula_name, suggest_homebrew, suggest_missing_formula_matches};

pub async fn execute(
    installer: &mut zb_io::Installer,
    formulas: Vec<String>,
    no_link: bool,
    build_from_source: bool,
    ui: &mut StdUi,
) -> Result<(), zb_core::Error> {
    let start = Instant::now();

    // 1. Determine which formulas we are upgrading.
    let mut upgrading_names = Vec::new();

    if formulas.is_empty() {
        ui.heading("Checking for outdated packages...").map_err(ui_error)?;
        let (outdated, warnings) = installer.check_outdated().await?;
        for warning in &warnings {
            ui.error(format!("Warning: {}", warning)).map_err(ui_error)?;
        }

        if outdated.is_empty() {
            ui.info("All packages are up to date.").map_err(ui_error)?;
            return Ok(());
        }

        for pkg in &outdated {
            upgrading_names.push(pkg.name.clone());
        }
    } else {
        // Only upgrade the requested formulas.
        for formula in &formulas {
            match normalize_formula_name(formula) {
                Ok(name) => {
                    if name.starts_with("cask:") {
                        ui.error(format!("Upgrading casks is not yet supported: {}", name)).map_err(ui_error)?;
                        continue;
                    }
                    
                    // Check if the specific package is outdated
                    match installer.is_outdated(&name).await {
                        Ok(Some(_)) => {
                            upgrading_names.push(name);
                        }
                        Ok(None) => {
                            ui.info(format!("{} is already up to date.", style(&name).bold())).map_err(ui_error)?;
                        }
                        Err(e @ zb_core::Error::NotInstalled { .. }) => {
                            ui.error(format!("{} is not installed.", style(&name).bold())).map_err(ui_error)?;
                            return Err(e);
                        }
                        Err(e) => {
                            ui.error(format!("Error checking if {} is outdated: {}", name, e)).map_err(ui_error)?;
                            return Err(e);
                        }
                    }
                }
                Err(e) => {
                    suggest_homebrew(formula, &e);
                    return Err(e);
                }
            }
        }
        
        if upgrading_names.is_empty() {
            return Ok(());
        }
    }
    
    ui.heading(format!(
        "Upgrading {}...",
        style(upgrading_names.join(", ")).bold()
    ))
    .map_err(ui_error)?;

    // 2. Generate install plan
    let plan = match installer
        .plan_with_options(&upgrading_names, build_from_source)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            let handled_missing = suggest_missing_formula_matches(installer, &e).await;

            if !handled_missing {
                for formula in &formulas {
                    suggest_homebrew(formula, &e);
                }
            }
            return Err(e);
        }
    };

    let mut installed_formulas = Vec::new();
    let currently_installed = installer.list_installed()?;
    for keg in currently_installed {
        installed_formulas.push(keg.name);
    }

    ui.heading(format!(
        "Resolving dependencies ({} packages)...",
        plan.items.len()
    ))
    .map_err(ui_error)?;
    
    for item in &plan.items {
        // Check if this package is being upgraded or newly installed as a dependency
        let status = if installed_formulas.contains(&item.formula.name) {
            style("upgrading").yellow()
        } else {
            style("installing").green()
        };
        
        ui.bullet(format!(
            "{} {} ({})",
            style(&item.formula.name).green(),
            style(&item.formula.versions.stable).dim(),
            status
        ))
        .map_err(ui_error)?;
    }

    // 3. Uninstall existing outdated packages from the plan before running install execution
    // to prevent LinkConflict.
    for item in &plan.items {
        if installed_formulas.contains(&item.formula.name) {
            ui.step_start(format!("Uninstalling old version of {}", item.formula.name)).map_err(ui_error)?;
            if let Err(e) = installer.uninstall(&item.formula.name) {
                ui.step_fail().map_err(ui_error)?;
                ui.error(format!("Failed to uninstall old version of {}: {}", item.formula.name, e)).map_err(ui_error)?;
                return Err(e);
            }
            ui.step_ok().map_err(ui_error)?;
        }
    }

    // 4. Download and install formulas with progress tracking
    let multi = MultiProgress::new();
    let bars: Arc<Mutex<HashMap<String, ProgressBar>>> = Arc::new(Mutex::new(HashMap::new()));

    let download_style = ProgressStyle::default_bar()
        .template("    {prefix:<16} {bar:25.cyan/dim} {bytes:>10}/{total_bytes:<10} {eta:>6}")
        .unwrap()
        .progress_chars("━━╸");

    let spinner_style = ProgressStyle::default_spinner()
        .template("    {prefix:<16} {spinner:.cyan} {msg}")
        .unwrap()
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏");

    let done_style = ProgressStyle::default_spinner()
        .template("    {prefix:<16} {msg}")
        .unwrap();

    ui.heading("Downloading and installing formulas...")
        .map_err(ui_error)?;

    let bars_clone = bars.clone();
    let multi_clone = multi.clone();
    let download_style_clone = download_style.clone();
    let spinner_style_clone = spinner_style.clone();
    let done_style_clone = done_style.clone();

    let progress_callback: Arc<ProgressCallback> = Arc::new(Box::new(move |event| {
        let mut bars = bars_clone.lock().unwrap();
        match event {
            InstallProgress::DownloadStarted { name, total_bytes } => {
                let pb = if let Some(total) = total_bytes {
                    let pb = multi_clone.add(ProgressBar::new(total));
                    pb.set_style(download_style_clone.clone());
                    pb
                } else {
                    let pb = multi_clone.add(ProgressBar::new_spinner());
                    pb.set_style(spinner_style_clone.clone());
                    pb.set_message("downloading...");
                    pb.enable_steady_tick(std::time::Duration::from_millis(80));
                    pb
                };
                pb.set_prefix(name.clone());
                bars.insert(name, pb);
            }
            InstallProgress::DownloadProgress {
                name,
                downloaded,
                total_bytes,
            } => {
                if let Some(pb) = bars.get(&name)
                    && total_bytes.is_some()
                {
                    pb.set_position(downloaded);
                }
            }
            InstallProgress::DownloadCompleted { name, total_bytes } => {
                if let Some(pb) = bars.get(&name) {
                    if total_bytes > 0 {
                        pb.set_position(total_bytes);
                    }
                    pb.set_style(spinner_style_clone.clone());
                    pb.set_message("unpacking...");
                    pb.enable_steady_tick(std::time::Duration::from_millis(80));
                }
            }
            InstallProgress::UnpackStarted { name } => {
                if let Some(pb) = bars.get(&name) {
                    pb.set_message("unpacking...");
                }
            }
            InstallProgress::UnpackCompleted { name } => {
                if let Some(pb) = bars.get(&name) {
                    pb.set_message("unpacked");
                }
            }
            InstallProgress::LinkStarted { name } => {
                if let Some(pb) = bars.get(&name) {
                    pb.set_message("linking...");
                }
            }
            InstallProgress::LinkCompleted { name } => {
                if let Some(pb) = bars.get(&name) {
                    pb.set_message("linked");
                }
            }
            InstallProgress::LinkSkipped { name, reason } => {
                if let Some(pb) = bars.get(&name) {
                    pb.set_message(format!("keg-only ({})", reason));
                }
            }
            InstallProgress::InstallCompleted { name } => {
                if let Some(pb) = bars.get(&name) {
                    pb.set_style(done_style_clone.clone());
                    pb.set_message(format!("{} installed", style("✓").green()));
                    pb.finish();
                }
            }
        }
    }));

    let result_val = installer
        .execute_with_progress(plan, !no_link, Some(progress_callback))
        .await;

    {
        let bars = bars.lock().unwrap();
        for (_, pb) in bars.iter() {
            if !pb.is_finished() {
                pb.finish();
            }
        }
    }

    let result = match result_val {
        Ok(r) => r,
        Err(ref e @ zb_core::Error::LinkConflict { ref conflicts }) => {
            ui.blank_line().map_err(ui_error)?;
            ui.error("The link step did not complete successfully.")
                .map_err(ui_error)?;
            ui.println("The formula was upgraded, but is not symlinked into the prefix.")
                .map_err(ui_error)?;
            ui.blank_line().map_err(ui_error)?;
            ui.println("Possible conflicting files:")
                .map_err(ui_error)?;
            for c in conflicts {
                if let Some(ref owner) = c.owned_by {
                    ui.println(format!(
                        "  {} (symlink belonging to {})",
                        c.path.display(),
                        style(owner).yellow()
                    ))
                    .map_err(ui_error)?;
                } else {
                    ui.println(format!("  {}", c.path.display()))
                        .map_err(ui_error)?;
                }
            }
            ui.blank_line().map_err(ui_error)?;
            return Err(e.clone());
        }
        Err(e) => {
            let handled_missing = suggest_missing_formula_matches(installer, &e).await;

            if !handled_missing {
                for formula in &formulas {
                    suggest_homebrew(formula, &e);
                }
            }
            return Err(e);
        }
    };

    let elapsed = start.elapsed();
    ui.blank_line().map_err(ui_error)?;
    ui.heading(format!(
        "Upgraded {} packages in {:.2}s",
        style(result.installed).green().bold(),
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
