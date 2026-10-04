//! Les parties : quel jeu Steam tourne sur ce PC, et depuis quand.
//!
//! Toutes les 30 secondes, l'app regarde les programmes lancés. Un programme
//! qui tourne depuis le dossier d'installation d'un jeu Steam
//! (`steamapps/common/<jeu>`) veut dire que ce jeu est lancé — pas besoin de
//! connaître le nom de son `.exe`. Quand il disparaît, la partie est finie et
//! part dans le carnet des parties à envoyer.
//!
//! Le carnet est écrit sur le disque à chaque changement : une partie finie
//! pendant que le site n'écoutait pas (page en train de charger, pas de
//! réseau) n'est pas perdue, et une partie en cours quand le PC s'éteint est
//! close à la dernière fois où on l'a vue tourner.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// En dessous d'une minute, c'est un lancement raté ou un launcher qui
/// clignote, pas une partie.
pub const DUREE_MINIMALE_MS: i64 = 60_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Partie {
    /// Identifiant stable côté PC (`appid-début`) : le site s'en sert pour
    /// ne jamais enregistrer deux fois la même partie.
    pub id: String,
    pub appid: u32,
    pub title: String,
    /// Millisecondes depuis 1970.
    pub started_at: i64,
    /// Pour une partie en cours : la dernière fois qu'on l'a vue tourner.
    pub ended_at: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Carnet {
    pub en_cours: Vec<Partie>,
    pub terminees: Vec<Partie>,
}

impl Carnet {
    /// Fait avancer le carnet d'un relevé. `vus` = les jeux qui tournent en
    /// ce moment. Rend `true` si au moins une partie vient de se terminer.
    pub fn avancer(&mut self, vus: &[(u32, String)], maintenant: i64) -> bool {
        let mut fini = false;
        let mut encore = Vec::new();
        for mut partie in std::mem::take(&mut self.en_cours) {
            if vus.iter().any(|(appid, _)| *appid == partie.appid) {
                partie.ended_at = maintenant;
                encore.push(partie);
            } else if partie.ended_at - partie.started_at >= DUREE_MINIMALE_MS {
                self.terminees.push(partie);
                fini = true;
            }
        }
        for (appid, title) in vus {
            if !encore.iter().any(|p| p.appid == *appid) {
                encore.push(Partie {
                    id: format!("{appid}-{maintenant}"),
                    appid: *appid,
                    title: title.clone(),
                    started_at: maintenant,
                    ended_at: maintenant,
                });
            }
        }
        self.en_cours = encore;
        fini
    }

    /// Au démarrage : ce qui était « en cours » quand l'app s'est arrêtée
    /// est fini, à la dernière fois où on l'a vu tourner.
    pub fn reprendre(&mut self) {
        for partie in std::mem::take(&mut self.en_cours) {
            if partie.ended_at - partie.started_at >= DUREE_MINIMALE_MS {
                self.terminees.push(partie);
            }
        }
    }

    /// Le site a enregistré ces parties : on les oublie.
    pub fn oublier(&mut self, ids: &[String]) {
        self.terminees.retain(|p| !ids.contains(&p.id));
    }
}

/// Le carnet partagé entre la boucle de surveillance et les commandes.
pub struct Parties {
    carnet: Mutex<Carnet>,
    fichier: Option<PathBuf>,
}

impl Parties {
    /// Relit le carnet laissé par la dernière session de l'app.
    pub fn charger(fichier: Option<PathBuf>) -> Self {
        let mut carnet: Carnet = fichier
            .as_deref()
            .and_then(|f| fs::read_to_string(f).ok())
            .and_then(|texte| serde_json::from_str(&texte).ok())
            .unwrap_or_default();
        carnet.reprendre();
        let parties = Parties {
            carnet: Mutex::new(carnet),
            fichier,
        };
        parties.ecrire();
        parties
    }

    /// Un relevé de la boucle. Rend `true` si une partie vient de se finir.
    pub fn relever(&self, vus: &[(u32, String)], maintenant: i64) -> bool {
        let (fini, a_ecrire) = {
            let mut carnet = self.carnet.lock().unwrap();
            let avant = carnet.en_cours.len();
            let fini = carnet.avancer(vus, maintenant);
            // Écrire aussi tant qu'un jeu tourne, pour que « la dernière fois
            // qu'on l'a vu » reste juste si le PC s'éteint d'un coup.
            (fini, fini || avant > 0 || !carnet.en_cours.is_empty())
        };
        if a_ecrire {
            self.ecrire();
        }
        fini
    }

    pub fn a_envoyer(&self) -> Vec<Partie> {
        self.carnet.lock().unwrap().terminees.clone()
    }

    pub fn en_cours(&self) -> Vec<Partie> {
        self.carnet.lock().unwrap().en_cours.clone()
    }

    pub fn envoyees(&self, ids: &[String]) {
        self.carnet.lock().unwrap().oublier(ids);
        self.ecrire();
    }

    fn ecrire(&self) {
        let Some(fichier) = &self.fichier else { return };
        let texte = {
            let carnet = self.carnet.lock().unwrap();
            serde_json::to_string(&*carnet)
        };
        if let Ok(texte) = texte {
            if let Some(dossier) = fichier.parent() {
                let _ = fs::create_dir_all(dossier);
            }
            // Écrire à côté puis renommer : un carnet à moitié écrit (PC
            // coupé au mauvais moment) ne remplace jamais le bon.
            let provisoire = fichier.with_extension("json.tmp");
            if fs::write(&provisoire, texte).is_ok() {
                let _ = fs::rename(&provisoire, fichier);
            }
        }
    }
}

/// Un chemin comparable : barres obliques, minuscules (Windows ignore la
/// casse), et une barre finale pour qu'un dossier « Foo » ne capte pas
/// « FooBar ».
pub fn normaliser(chemin: &Path) -> String {
    let mut texte = chemin.to_string_lossy().replace('\\', "/").to_lowercase();
    if !texte.ends_with('/') {
        texte.push('/');
    }
    texte
}

/// Les jeux qui tournent : ceux dont un programme vit dans leur dossier.
/// `dossiers` = (appid, titre, dossier déjà normalisé).
pub fn jeux_qui_tournent(systeme: &mut System, dossiers: &[(u32, String, String)]) -> Vec<(u32, String)> {
    systeme.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    let chemins: Vec<String> = systeme
        .processes()
        .values()
        .filter_map(|p| p.exe())
        .map(normaliser)
        .collect();
    reconnaitre(&chemins, dossiers)
}

/// La partie pure de la reconnaissance, testable sans vrais programmes.
pub fn reconnaitre(chemins: &[String], dossiers: &[(u32, String, String)]) -> Vec<(u32, String)> {
    let mut vus: Vec<(u32, String)> = Vec::new();
    for (appid, titre, dossier) in dossiers {
        // Un dossier vide (manifeste sans installdir) serait le dossier
        // `common` entier : il reconnaîtrait n'importe quel jeu.
        if dossier.ends_with("/common/") {
            continue;
        }
        if chemins.iter().any(|c| c.starts_with(dossier.as_str())) && !vus.iter().any(|(a, _)| a == appid) {
            vus.push((*appid, titre.clone()));
        }
    }
    vus
}

pub fn maintenant_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn vu(appid: u32) -> (u32, String) {
        (appid, format!("Jeu {appid}"))
    }

    #[test]
    fn une_partie_complete() {
        let mut c = Carnet::default();
        assert!(!c.avancer(&[vu(10)], 0));
        assert!(!c.avancer(&[vu(10)], 30 * MIN));
        assert!(c.avancer(&[], 31 * MIN));
        assert_eq!(c.terminees.len(), 1);
        let p = &c.terminees[0];
        assert_eq!((p.id.as_str(), p.started_at, p.ended_at), ("10-0", 0, 30 * MIN));
        assert!(c.en_cours.is_empty());
    }

    #[test]
    fn un_lancement_rate_ne_compte_pas() {
        let mut c = Carnet::default();
        c.avancer(&[vu(10)], 0);
        c.avancer(&[vu(10)], 30_000);
        assert!(!c.avancer(&[], 60_000));
        assert!(c.terminees.is_empty());
    }

    #[test]
    fn deux_jeux_en_meme_temps() {
        let mut c = Carnet::default();
        c.avancer(&[vu(10), vu(20)], 0);
        c.avancer(&[vu(10), vu(20)], 10 * MIN);
        assert!(c.avancer(&[vu(20)], 11 * MIN));
        assert_eq!(c.terminees.len(), 1);
        assert_eq!(c.en_cours.len(), 1);
        assert_eq!(c.en_cours[0].appid, 20);
    }

    #[test]
    fn reprendre_clot_ce_qui_tournait() {
        let mut c = Carnet::default();
        c.avancer(&[vu(10)], 0);
        c.avancer(&[vu(10)], 45 * MIN);
        c.reprendre();
        assert_eq!(c.terminees[0].ended_at, 45 * MIN);
        assert!(c.en_cours.is_empty());
    }

    #[test]
    fn oublier_les_parties_envoyees() {
        let mut c = Carnet::default();
        c.avancer(&[vu(10)], 0);
        c.avancer(&[vu(10)], 5 * MIN);
        c.avancer(&[], 6 * MIN);
        c.oublier(&["10-0".to_string()]);
        assert!(c.terminees.is_empty());
    }

    #[test]
    fn reconnaitre_par_le_dossier() {
        let dossiers = vec![
            (10, "Foo".to_string(), normaliser(Path::new("D:\\SteamLibrary\\steamapps\\common\\Foo"))),
            (20, "FooBar".to_string(), normaliser(Path::new("D:\\SteamLibrary\\steamapps\\common\\FooBar"))),
            (30, "Vide".to_string(), normaliser(Path::new("D:\\SteamLibrary\\steamapps\\common\\"))),
        ];
        let chemins = vec![
            normaliser(Path::new("d:\\steamlibrary\\steamapps\\common\\FooBar\\bin\\game.exe")),
            normaliser(Path::new("C:\\Windows\\explorer.exe")),
        ];
        assert_eq!(reconnaitre(&chemins, &dossiers), vec![(20, "FooBar".to_string())]);
    }

    #[test]
    fn le_carnet_survit_a_un_redemarrage() {
        let fichier = std::env::temp_dir().join(format!("squad-parties-{}.json", std::process::id()));
        let parties = Parties::charger(Some(fichier.clone()));
        parties.relever(&[vu(10)], 0);
        parties.relever(&[vu(10)], 20 * MIN);
        drop(parties);
        let reprises = Parties::charger(Some(fichier.clone()));
        let a_envoyer = reprises.a_envoyer();
        assert_eq!(a_envoyer.len(), 1);
        assert_eq!(a_envoyer[0].ended_at, 20 * MIN);
        reprises.envoyees(&[a_envoyer[0].id.clone()]);
        assert!(Parties::charger(Some(fichier.clone())).a_envoyer().is_empty());
        let _ = fs::remove_file(fichier);
    }

}
