//! Les réglages de l'app qui doivent survivre à un redémarrage : le mode
//! invisible (le site n'annonce plus à la bande à quoi on joue) et
//! l'ouverture automatique du compagnon LoL avec le jeu.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Contenu {
    #[serde(default)]
    invisible: bool,
    /// Ouvrir le compagnon LoL tout seul quand le client LoL démarre.
    #[serde(default = "oui")]
    compagnon_auto: bool,
}

fn oui() -> bool {
    true
}

impl Default for Contenu {
    fn default() -> Self {
        Contenu { invisible: false, compagnon_auto: true }
    }
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

    pub fn compagnon_auto(&self) -> bool {
        self.contenu.lock().unwrap().compagnon_auto
    }

    pub fn basculer_compagnon_auto(&self) -> bool {
        let nouveau = {
            let mut contenu = self.contenu.lock().unwrap();
            contenu.compagnon_auto = !contenu.compagnon_auto;
            contenu.compagnon_auto
        };
        self.ecrire();
        nouveau
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
