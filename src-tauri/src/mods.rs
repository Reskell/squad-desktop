//! Le Garage en un clic (SQUAD//JOIN, vague 5) : installer le mod d'un jeu
//! sans toucher un fichier.
//!
//! Le site donne un paquet : une archive .zip déposée par la bande, son
//! empreinte SHA-256 et le sous-dossier du jeu où la décompresser. L'app :
//!
//! 1. télécharge l'archive (adresse signée, valable quelques minutes) ;
//! 2. refuse tout fichier dont l'empreinte ne correspond pas — personne ne
//!    peut glisser autre chose à la place du paquet déposé ;
//! 3. décompresse dans le dossier du jeu, sans jamais en sortir (un chemin
//!    d'archive qui remonte avec « .. » est refusé) ;
//! 4. garde une copie de chaque fichier qu'elle remplace : désinstaller les
//!    remet, et retire ce qu'elle avait ajouté.
//!
//! La liste de ce qui est installé vit dans `mods.json`, les copies dans
//! `mods/<paquet>/`, à côté des autres réglages de l'app.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Au-delà, on ne télécharge pas (le site refuse déjà plus de 50 Mo).
pub const TAILLE_MAX: u64 = 60 * 1024 * 1024;

/// Ce que le site demande d'installer.
#[derive(Deserialize, Clone)]
pub struct Paquet {
    pub id: String,
    pub game_id: String,
    pub nom: String,
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub taille: u64,
    pub cible: String,
    pub appid: Option<u32>,
}

/// Ce que l'app retient d'une installation.
#[derive(Serialize, Deserialize, Clone)]
struct Installe {
    id: String,
    game_id: String,
    nom: String,
    version: String,
    dossier: String,
    /// Les fichiers écrits, relatifs au dossier du jeu (avec des « / »).
    fichiers: Vec<String>,
    /// Ceux qui existaient avant et dont une copie est gardée.
    sauvegardes: Vec<String>,
    /// Secondes depuis 1970.
    le: u64,
}

/// Ce que le site voit d'une installation.
#[derive(Serialize, Clone)]
pub struct Visible {
    pub id: String,
    pub game_id: String,
    pub nom: String,
    pub version: String,
    pub dossier: String,
    pub fichiers: usize,
    pub le: String,
}

impl From<&Installe> for Visible {
    fn from(i: &Installe) -> Self {
        Visible {
            id: i.id.clone(),
            game_id: i.game_id.clone(),
            nom: i.nom.clone(),
            version: i.version.clone(),
            dossier: i.dossier.clone(),
            fichiers: i.fichiers.len(),
            le: i.le.to_string(),
        }
    }
}

pub struct Mods {
    fichier: Option<PathBuf>,
    copies: Option<PathBuf>,
    liste: Mutex<Vec<Installe>>,
}

/// Un sous-dossier « sûr » : relatif, sans « .. », sans racine ni lecteur.
pub fn chemin_sur(texte: &str) -> Option<PathBuf> {
    let mut chemin = PathBuf::new();
    for morceau in texte.replace('\\', "/").split('/') {
        if morceau.is_empty() || morceau == "." {
            continue;
        }
        if morceau == ".." || morceau.contains(':') {
            return None;
        }
        chemin.push(morceau);
    }
    if chemin.components().all(|c| matches!(c, Component::Normal(_))) {
        Some(chemin)
    } else {
        None
    }
}

fn en_texte(chemin: &Path) -> String {
    chemin.to_string_lossy().replace('\\', "/")
}

pub fn empreinte(octets: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(octets);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Télécharge l'archive d'un paquet.
pub async fn telecharger(url: &str, taille_annoncee: u64) -> Result<Vec<u8>, String> {
    if !url.starts_with("https://") {
        return Err("Adresse du paquet refusée.".into());
    }
    if taille_annoncee > TAILLE_MAX {
        return Err("Paquet trop gros.".into());
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|_| "Téléchargement impossible.".to_string())?;
    let reponse = client
        .get(url)
        .send()
        .await
        .map_err(|_| "Le site ne répond pas : réessaie dans un instant.".to_string())?;
    if !reponse.status().is_success() {
        return Err("Le lien du paquet a expiré : réessaie.".into());
    }
    let octets = reponse.bytes().await.map_err(|_| "Téléchargement interrompu.".to_string())?;
    if octets.len() as u64 > TAILLE_MAX {
        return Err("Paquet trop gros.".into());
    }
    Ok(octets.to_vec())
}

impl Mods {
    pub fn charger(dossier_app: Option<PathBuf>) -> Self {
        let fichier = dossier_app.as_ref().map(|d| d.join("mods.json"));
        let liste = fichier
            .as_ref()
            .and_then(|f| fs::read_to_string(f).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Mods {
            fichier,
            copies: dossier_app.map(|d| d.join("mods")),
            liste: Mutex::new(liste),
        }
    }

    fn enregistrer(&self, liste: &[Installe]) {
        if let Some(f) = &self.fichier {
            if let Some(parent) = f.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(texte) = serde_json::to_string_pretty(liste) {
                let _ = fs::write(f, texte);
            }
        }
    }

    pub fn visibles(&self) -> Vec<Visible> {
        self.liste.lock().unwrap().iter().map(Visible::from).collect()
    }

    fn dossier_des_copies(&self, id: &str) -> Result<PathBuf, String> {
        let racine = self.copies.clone().ok_or("Dossier de l'app introuvable.")?;
        let id_sur = chemin_sur(id).ok_or("Paquet refusé.")?;
        Ok(racine.join(id_sur))
    }

    /// Décompresse une archive vérifiée dans le dossier du jeu.
    pub fn installer(&self, paquet: &Paquet, octets: &[u8], dossier_jeu: &Path) -> Result<Visible, String> {
        if empreinte(octets) != paquet.sha256.to_lowercase() {
            return Err("Le fichier téléchargé ne correspond pas au paquet déposé : installation refusée.".into());
        }
        if !dossier_jeu.is_dir() {
            return Err("Le dossier du jeu est introuvable.".into());
        }
        let cible = chemin_sur(&paquet.cible).ok_or("Sous-dossier refusé.")?;
        // Une ancienne version du même paquet : on la retire d'abord.
        if self.liste.lock().unwrap().iter().any(|i| i.id == paquet.id) {
            self.desinstaller(&paquet.id)?;
        }
        let copies = self.dossier_des_copies(&paquet.id)?;
        let mut archive =
            zip::ZipArchive::new(Cursor::new(octets)).map_err(|_| "Ce paquet n'est pas une archive .zip lisible.".to_string())?;

        let mut fichiers: Vec<String> = Vec::new();
        let mut sauvegardes: Vec<String> = Vec::new();
        let resultat = (|| -> Result<(), String> {
            for i in 0..archive.len() {
                let mut entree = archive.by_index(i).map_err(|_| "Archive abîmée.".to_string())?;
                // enclosed_name refuse les chemins qui sortent de l'archive.
                let nom = entree.enclosed_name().ok_or("Un fichier de l'archive veut sortir du dossier du jeu.")?;
                let relatif = cible.join(&nom);
                let destination = dossier_jeu.join(&relatif);
                if entree.is_dir() {
                    fs::create_dir_all(&destination).map_err(|e| format!("Dossier impossible à créer : {e}"))?;
                    continue;
                }
                let cle = en_texte(&relatif);
                if destination.exists() && !sauvegardes.contains(&cle) {
                    let copie = copies.join(&relatif);
                    if let Some(parent) = copie.parent() {
                        fs::create_dir_all(parent).map_err(|e| format!("Copie impossible : {e}"))?;
                    }
                    fs::copy(&destination, &copie).map_err(|e| format!("Copie impossible : {e}"))?;
                    sauvegardes.push(cle.clone());
                }
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent).map_err(|e| format!("Dossier impossible à créer : {e}"))?;
                }
                let mut contenu = Vec::new();
                entree.read_to_end(&mut contenu).map_err(|_| "Archive abîmée.".to_string())?;
                fs::write(&destination, contenu).map_err(|e| {
                    format!("Écriture impossible (le jeu est-il ouvert ?) : {e}")
                })?;
                if !fichiers.contains(&cle) {
                    fichiers.push(cle);
                }
            }
            Ok(())
        })();

        let installe = Installe {
            id: paquet.id.clone(),
            game_id: paquet.game_id.clone(),
            nom: paquet.nom.clone(),
            version: paquet.version.clone(),
            dossier: en_texte(dossier_jeu),
            fichiers,
            sauvegardes,
            le: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        };
        if let Err(erreur) = resultat {
            // À mi-chemin : on remet tout comme avant.
            let _ = remettre(&installe, &copies);
            return Err(erreur);
        }
        let visible = Visible::from(&installe);
        let mut liste = self.liste.lock().unwrap();
        liste.push(installe);
        self.enregistrer(&liste);
        Ok(visible)
    }

    /// Retire ce qu'un paquet avait écrit et remet les fichiers d'origine.
    pub fn desinstaller(&self, id: &str) -> Result<(), String> {
        let installe = {
            let liste = self.liste.lock().unwrap();
            liste.iter().find(|i| i.id == id).cloned()
        }
        .ok_or("Ce paquet n'est pas installé ici.")?;
        let copies = self.dossier_des_copies(id)?;
        remettre(&installe, &copies)?;
        let _ = fs::remove_dir_all(&copies);
        let mut liste = self.liste.lock().unwrap();
        liste.retain(|i| i.id != id);
        self.enregistrer(&liste);
        Ok(())
    }
}

/// Efface les fichiers écrits, puis recopie les originaux gardés.
fn remettre(installe: &Installe, copies: &Path) -> Result<(), String> {
    let dossier = PathBuf::from(&installe.dossier);
    for f in &installe.fichiers {
        if let Some(rel) = chemin_sur(f) {
            let _ = fs::remove_file(dossier.join(rel));
        }
    }
    for f in &installe.sauvegardes {
        if let Some(rel) = chemin_sur(f) {
            fs::copy(copies.join(&rel), dossier.join(&rel))
                .map_err(|e| format!("Impossible de remettre {f} (le jeu est-il ouvert ?) : {e}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn archive(fichiers: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tampon = Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut tampon);
            let options: zip::write::SimpleFileOptions =
                zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (nom, contenu) in fichiers {
                z.start_file(*nom, options).unwrap();
                z.write_all(contenu).unwrap();
            }
            z.finish().unwrap();
        }
        tampon.into_inner()
    }

    fn paquet(octets: &[u8], cible: &str) -> Paquet {
        Paquet {
            id: "p1".into(),
            game_id: "g1".into(),
            nom: "Mod".into(),
            version: "1.0".into(),
            url: String::new(),
            sha256: empreinte(octets),
            taille: octets.len() as u64,
            cible: cible.into(),
            appid: None,
        }
    }

    #[test]
    fn chemins() {
        assert_eq!(chemin_sur(""), Some(PathBuf::new()));
        assert_eq!(chemin_sur("BepInEx/plugins"), Some(PathBuf::from("BepInEx").join("plugins")));
        assert_eq!(chemin_sur("a\\b"), Some(PathBuf::from("a").join("b")));
        assert!(chemin_sur("../x").is_none());
        assert!(chemin_sur("a/../../x").is_none());
        assert!(chemin_sur("C:/Windows").is_none());
        assert_eq!(chemin_sur("/etc"), Some(PathBuf::from("etc")));
    }

    #[test]
    fn installer_puis_desinstaller() {
        let base = std::env::temp_dir().join(format!("squad-mods-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let jeu = base.join("jeu");
        fs::create_dir_all(jeu.join("data")).unwrap();
        fs::write(jeu.join("data").join("a.txt"), b"original").unwrap();
        let mods = Mods::charger(Some(base.join("app")));

        let octets = archive(&[("a.txt", b"mod"), ("sous/b.dll", b"dll")]);
        let p = paquet(&octets, "data");
        let v = mods.installer(&p, &octets, &jeu).unwrap();
        assert_eq!(v.fichiers, 2);
        assert_eq!(fs::read(jeu.join("data").join("a.txt")).unwrap(), b"mod");
        assert_eq!(fs::read(jeu.join("data").join("sous").join("b.dll")).unwrap(), b"dll");
        // Relu depuis le disque.
        assert_eq!(Mods::charger(Some(base.join("app"))).visibles().len(), 1);

        mods.desinstaller("p1").unwrap();
        assert_eq!(fs::read(jeu.join("data").join("a.txt")).unwrap(), b"original");
        assert!(!jeu.join("data").join("sous").join("b.dll").exists());
        assert!(mods.visibles().is_empty());

        // Une empreinte fausse : rien n'est écrit.
        let mut faux = paquet(&octets, "");
        faux.sha256 = "0".repeat(64);
        assert!(mods.installer(&faux, &octets, &jeu).is_err());
        assert!(!jeu.join("a.txt").exists());

        // Une archive qui veut sortir du dossier.
        let mechante = archive(&[("../evade.txt", b"x")]);
        assert!(mods.installer(&paquet(&mechante, ""), &mechante, &jeu).is_err());
        assert!(!base.join("evade.txt").exists());
        let _ = fs::remove_dir_all(&base);
    }
}
