fn main() {
    // Déclarer les commandes de l'app les soumet aux droits de
    // capabilities/default.json : sans ça, n'importe quelle page ouverte dans
    // la fenêtre (une fiche Steam suivie depuis le site, par exemple) pourrait
    // les appeler. Là, seul le site SQUAD//LOG y a droit.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&[
                "jeux_installes",
                "parties_a_envoyer",
                "parties_envoyees",
                "parties_en_cours",
                "presence_invisible",
                "jeux_hors_steam",
                "choisir_programme",
                "associer_jeu_hors_steam",
                "retirer_jeu_hors_steam",
                "lancer_jeu_hors_steam",
            ])),
    )
    .expect("échec de la préparation de la compilation");
}
