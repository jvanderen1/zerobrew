use console::style;

pub fn execute(installer: &mut zb_io::Installer) -> Result<(), zb_core::Error> {
    println!(
        "{} Running garbage collection...",
        style("==>").cyan().bold()
    );
    let removed = installer.gc()?;

    if removed.is_empty() {
        println!("No unreferenced store entries to remove.");
    } else {
        for key in &removed {
            println!("    {} Removed {}", style("✓").green(), &key[..12]);
        }
        println!(
            "{} Removed {} store entries",
            style("==>").cyan().bold(),
            style(removed.len()).green().bold()
        );
    }

    // Clean up orphaned cellar kegs (old versions left behind by upgrades)
    let orphans = installer.cleanup_orphaned_kegs()?;

    if !orphans.is_empty() {
        for (name, version) in &orphans {
            println!(
                "    {} Removed old keg {}/{}",
                style("✓").green(),
                name,
                version
            );
        }
        println!(
            "{} Removed {} orphaned cellar {}",
            style("==>").cyan().bold(),
            style(orphans.len()).green().bold(),
            if orphans.len() == 1 { "keg" } else { "kegs" }
        );
    }

    Ok(())
}
