//! Les réglages de l'app qui doivent survivre à un redémarrage. Pour
//! l'instant un seul : le mode invisible, qui empêche le site d'annoncer à
//! la bande à quoi on joue.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Contenu {
    #[serde(default)]
    invisible: bool,
}

pub struct Reglages {
    contenu: Mutex<Contenu>,
    fichier: Option<PathBuf>,
}

impl Reglages {
    pub fn charger(fichier: Option<PathBuf>) -> Self {
        let contenu = fichier
            .as_deref()
            .and_then(|f| fs::read_to_string(f).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Reglages { contenu: Mutex::new(contenu), fichier }
    }

    pub fn invisible(&self) -> bool {
        self.contenu.lock().unwrap().invisible
    }

    /// Bascule le mode invisible et rend le nouvel état.
    pub fn basculer_invisible(&self) -> bool {
        let nouveau = {
            let mut contenu = self.contenu.lock().unwrap();
            contenu.invisible = !contenu.invisible;
            contenu.invisible
        };
        self.ecrire();
        nouveau
    }

    fn ecrire(&self) {
        let Some(fichier) = &self.fichier else { return };
        let Ok(texte) = serde_json::to_string(&*self.contenu.lock().unwrap()) else {
            return;
        };
        if let Some(dossier) = fichier.parent() {
            let _ = fs::create_dir_all(dossier);
        }
        let _ = fs::write(fichier, texte);
    }
}
