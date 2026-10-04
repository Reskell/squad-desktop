fn main() {
    // Déclarer les commandes de l'app les soumet aux droits de
    // capabilities/default.json : sans ça, n'importe quelle page ouverte dans
    // la fenêtre (une fiche Steam suivie depuis le site, par exemple) pourrait
    // les appeler. Là, seul le site SQUAD//LOG y a droit.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["jeux_installes"])),
    )
    .expect("échec de la préparation de la compilation");
}
