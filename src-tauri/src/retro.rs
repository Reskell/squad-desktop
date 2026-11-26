//! SQUAD//RETRO dans l'app (vague 1) : les jeux rétro de la personne, lancés
//! en un clic avec RetroArch caché.
//!
//! Le moteur (dépôt Squad-retro) fait le travail : scanner le dossier des
//! jeux, reconnaître chaque fichier, préparer la commande. Ici, l'app garde
//! les deux réglages de la personne (le dossier des jeux, l'emplacement de
//! RetroArch) et ne lance que des fichiers trouvés par le dernier scan : une
//! page ne peut pas faire exécuter un chemin de son choix.
//!
//! Tout vit dans `retro.json` et `retro/` (configs, sauvegardes), à côté des
//! autres réglages de l'app.

use serde::{Deserialize, Serialize};
use squad_retro::bibliotheque::{self, Bibliotheque, Options};
use squad_retro::region::ORDRE_PAR_DEFAUT;
use squad_retro::{lancement, Console, Emplacements, Region};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Ce que la personne a choisi.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReglagesRetro {
    /// Le dossier qui contient `nes/`, `snes/`… (et `dats/` pour les bases).
    pub dossier_jeux: Option<String>,
    /// L'exécutable RetroArch.
    pub retroarch: Option<String>,
}

/// Une version d'un jeu, telle que la page la voit.
#[derive(Serialize, Clone)]
pub struct VarianteVisible {
    pub chemin: String,
    pub nom: String,
    pub region: Region,
    pub reconnu: bool,
}

/// Un jeu, tel que la page le voit.
#[derive(Serialize, Clone)]
pub struct FicheVisible {
    pub console: Console,
    pub console_nom: &'static str,
    pub titre: String,
    pub variantes: Vec<VarianteVisible>,
}

pub struct Retro {
    reglages: Mutex<ReglagesRetro>,
    fichier: Option<PathBuf>,
    /// `<données de l'app>/retro` : configs RetroArch et sauvegardes.
    donnees: PathBuf,
    derniere: Mutex<Bibliotheque>,
}

impl Retro {
    pub fn charger(donnees_app: Option<PathBuf>) -> Self {
        let fichier = donnees_app.as_ref().map(|d| d.join("retro.json"));
        let reglages = fichier
            .as_deref()
            .and_then(|f| fs::read_to_string(f).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let donnees = donnees_app.unwrap_or_else(|| PathBuf::from(".")).join("retro");
        Retro {
            reglages: Mutex::new(reglages),
            fichier,
            donnees,
            derniere: Mutex::new(Bibliotheque::default()),
        }
    }

    pub fn reglages(&self) -> ReglagesRetro {
        self.reglages.lock().unwrap().clone()
    }

    pub fn choisir_dossier_jeux(&self, dossier: &Path) {
        self.reglages.lock().unwrap().dossier_jeux = Some(dossier.to_string_lossy().into_owned());
        self.ecrire();
    }

    pub fn choisir_retroarch(&self, exe: &Path) {
        self.reglages.lock().unwrap().retroarch = Some(exe.to_string_lossy().into_owned());
        self.ecrire();
    }

    fn ecrire(&self) {
        let Some(fichier) = &self.fichier else { return };
        if let Some(dossier) = fichier.parent() {
            let _ = fs::create_dir_all(dossier);
        }
        if let Ok(texte) = serde_json::to_string_pretty(&*self.reglages.lock().unwrap()) {
            let _ = fs::write(fichier, texte);
        }
    }

    /// Scanne le dossier des jeux (lent : à appeler hors du fil de la fenêtre).
    pub fn scanner(&self) -> Result<Vec<FicheVisible>, String> {
        let dossier = self
            .reglages()
            .dossier_jeux
            .ok_or_else(|| "Choisis d'abord le dossier de tes jeux.".to_string())?;
        let racine = PathBuf::from(dossier);
        if !racine.is_dir() {
            return Err(format!("Le dossier {} est introuvable.", racine.display()));
        }
        let bases = bibliotheque::charger_bases(&racine.join("dats"));
        let biblio = Bibliotheque::scanner(&racine, &Options { bases: &bases, ordre: &ORDRE_PAR_DEFAUT });
        let visibles = biblio.fiches.iter().map(visible).collect();
        *self.derniere.lock().unwrap() = biblio;
        Ok(visibles)
    }

    /// Lance une version d'un jeu trouvée par le dernier scan.
    pub fn lancer(&self, chemin: &str) -> Result<(), String> {
        let (console, variante) = {
            let biblio = self.derniere.lock().unwrap();
            biblio
                .fiches
                .iter()
                .flat_map(|f| f.variantes.iter().map(move |v| (f.console, v)))
                .find(|(_, v)| v.chemin.to_string_lossy() == chemin)
                .map(|(c, v)| (c, v.clone()))
                .ok_or_else(|| "Ce jeu n'est plus dans la bibliothèque : relance le scan.".to_string())?
        };
        let retroarch = self
            .reglages()
            .retroarch
            .ok_or_else(|| "Choisis d'abord retroarch.exe dans les réglages.".to_string())?;
        let ou = Emplacements::depuis_retroarch(Path::new(&retroarch), &self.donnees);

        lancement::verifier(console, &ou)?;
        lancement::preparer(console, &ou, &self.donnees.join("sauvegardes"))
            .map_err(|e| format!("Impossible d'écrire la config de {} : {e}", console.nom()))?;

        let commande = lancement::commande(console, &variante, &ou);
        let mut process = commande.vers_process();
        if let Some(dossier) = commande.programme.parent() {
            process.current_dir(dossier);
        }
        process
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Impossible de lancer {} : {e}", commande.programme.display()))
    }
}

fn visible(fiche: &squad_retro::Fiche) -> FicheVisible {
    FicheVisible {
        console: fiche.console,
        console_nom: fiche.console.nom(),
        titre: fiche.titre.clone(),
        variantes: fiche
            .variantes
            .iter()
            .map(|v| VarianteVisible {
                chemin: v.chemin.to_string_lossy().into_owned(),
                nom: v.nom.clone(),
                region: v.region,
                reconnu: v.reconnu,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ne_lance_que_les_jeux_scannes() {
        let dir = std::env::temp_dir().join(format!("squad-retro-test-{}", std::process::id()));
        let snes = dir.join("jeux").join("snes");
        fs::create_dir_all(&snes).unwrap();
        fs::write(snes.join("Jeu (France).sfc"), b"x").unwrap();

        let retro = Retro::charger(Some(dir.join("app")));
        assert!(retro.lancer("C:/Windows/System32/calc.exe").is_err());

        retro.choisir_dossier_jeux(&dir.join("jeux"));
        let fiches = retro.scanner().unwrap();
        assert_eq!(fiches.len(), 1);
        assert_eq!(fiches[0].titre, "Jeu");

        // Trouvé par le scan, mais RetroArch n'est pas choisi : message clair.
        let erreur = retro.lancer(&fiches[0].variantes[0].chemin).unwrap_err();
        assert!(erreur.contains("retroarch"));

        // Les réglages survivent à un redémarrage.
        let relu = Retro::charger(Some(dir.join("app")));
        assert!(relu.reglages().dossier_jeux.is_some());
        let _ = fs::remove_dir_all(&dir);
    }
}
