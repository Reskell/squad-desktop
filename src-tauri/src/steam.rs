//! Les jeux Steam installés sur ce PC.
//!
//! Tout se lit sur le disque, sans réseau ni compte : Steam tient la liste
//! de ses bibliothèques dans `steamapps/libraryfolders.vdf`, et un fichier
//! `appmanifest_<appid>.acf` par jeu installé dans chacune. On n'en garde
//! que l'appid et le nom pour le site — jamais les chemins ni le reste du
//! PC (règle « confiance des potes » du document maître). Le dossier
//! d'installation reste dans l'app, pour reconnaître un jeu qui tourne.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct JeuInstalle {
    pub appid: u32,
    pub title: String,
}

/// Ce que Steam installe comme un jeu mais qui n'en est pas un.
const PAS_DES_JEUX: &[u32] = &[
    228980, // Steamworks Common Redistributables
    250820, // SteamVR
];

/// Tous les jeux installés, triés par titre, sans doublon d'appid.
/// Une liste vide si Steam est introuvable : ce n'est pas une erreur.
pub fn jeux_installes() -> Vec<JeuInstalle> {
    jeux_et_dossiers().into_iter().map(|(jeu, _)| jeu).collect()
}

/// Les jeux installés avec leur dossier d'installation, pour reconnaître un
/// jeu qui tourne. Le dossier ne quitte jamais l'app : il ne sert qu'ici.
pub fn jeux_et_dossiers() -> Vec<(JeuInstalle, PathBuf)> {
    match dossier_steam() {
        Some(racine) => jeux_dans(&racine),
        None => Vec::new(),
    }
}

/// Les jeux d'une installation Steam donnée, bibliothèques secondaires comprises.
pub fn jeux_dans(racine: &Path) -> Vec<(JeuInstalle, PathBuf)> {
    let mut bibliotheques = vec![racine.to_path_buf()];
    if let Ok(texte) = fs::read_to_string(racine.join("steamapps").join("libraryfolders.vdf")) {
        for chemin in valeurs(&texte, "path") {
            let dossier = PathBuf::from(chemin);
            if !bibliotheques.iter().any(|b| meme_dossier(b, &dossier)) {
                bibliotheques.push(dossier);
            }
        }
    }

    let mut jeux: Vec<(JeuInstalle, PathBuf)> = Vec::new();
    for bibliotheque in bibliotheques {
        // Un disque débranché ou une bibliothèque supprimée : on passe.
        let Ok(entrees) = fs::read_dir(bibliotheque.join("steamapps")) else {
            continue;
        };
        for entree in entrees.flatten() {
            let nom = entree.file_name().to_string_lossy().to_string();
            if !(nom.starts_with("appmanifest_") && nom.ends_with(".acf")) {
                continue;
            }
            let Ok(texte) = fs::read_to_string(entree.path()) else {
                continue;
            };
            if let Some((jeu, installdir)) = lire_manifeste_complet(&texte) {
                if !jeux.iter().any(|(j, _)| j.appid == jeu.appid) {
                    let dossier = bibliotheque.join("steamapps").join("common").join(installdir);
                    jeux.push((jeu, dossier));
                }
            }
        }
    }
    jeux.sort_by_key(|(j, _)| j.title.to_lowercase());
    jeux
}

/// Où Steam est installé : le registre d'abord, les emplacements habituels ensuite.
fn dossier_steam() -> Option<PathBuf> {
    let mut candidats: Vec<PathBuf> = Vec::new();

    #[cfg(windows)]
    {
        use winreg::{HKCU, HKLM};
        if let Ok(cle) = HKCU.open_subkey("Software\\Valve\\Steam") {
            if let Ok(chemin) = cle.get_value::<String, _>("SteamPath") {
                candidats.push(PathBuf::from(chemin));
            }
        }
        for sous_cle in ["SOFTWARE\\WOW6432Node\\Valve\\Steam", "SOFTWARE\\Valve\\Steam"] {
            if let Ok(cle) = HKLM.open_subkey(sous_cle) {
                if let Ok(chemin) = cle.get_value::<String, _>("InstallPath") {
                    candidats.push(PathBuf::from(chemin));
                }
            }
        }
        candidats.push(PathBuf::from("C:\\Program Files (x86)\\Steam"));
        candidats.push(PathBuf::from("C:\\Program Files\\Steam"));
    }

    #[cfg(not(windows))]
    {
        if let Some(maison) = std::env::var_os("HOME") {
            candidats.push(PathBuf::from(&maison).join(".steam/steam"));
            candidats.push(PathBuf::from(&maison).join(".local/share/Steam"));
        }
    }

    candidats.into_iter().find(|c| c.join("steamapps").is_dir())
}

/// Deux écritures du même dossier (casse, barres obliques) comptent pour un.
fn meme_dossier(a: &Path, b: &Path) -> bool {
    let normal = |p: &Path| {
        p.to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase()
    };
    normal(a) == normal(b)
}

/// Un manifeste de jeu → le jeu, s'il est entièrement installé et que c'en est un.
#[cfg(test)]
fn lire_manifeste(texte: &str) -> Option<JeuInstalle> {
    lire_manifeste_complet(texte).map(|(jeu, _)| jeu)
}

/// Le jeu et le nom de son dossier dans `steamapps/common`.
fn lire_manifeste_complet(texte: &str) -> Option<(JeuInstalle, String)> {
    let paires = paires(texte);
    let valeur = |cle: &str| {
        paires
            .iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(cle))
            .map(|(_, v)| v.as_str())
    };

    let appid: u32 = valeur("appid")?.trim().parse().ok()?;
    if PAS_DES_JEUX.contains(&appid) {
        return None;
    }
    // Le bit 4 de StateFlags = « entièrement installé ». Un jeu en cours de
    // téléchargement ou à moitié désinstallé n'est pas encore (ou plus) là.
    if let Some(drapeaux) = valeur("StateFlags").and_then(|f| f.trim().parse::<u32>().ok()) {
        if drapeaux & 4 == 0 {
            return None;
        }
    }
    let title = valeur("name")?.trim();
    if title.is_empty() {
        return None;
    }
    let installdir = valeur("installdir").map(str::trim).unwrap_or_default().to_string();
    Some((
        JeuInstalle {
            appid,
            title: title.to_string(),
        },
        installdir,
    ))
}

/// Toutes les valeurs d'une clé, où qu'elle soit dans le fichier.
fn valeurs(texte: &str, cle: &str) -> Vec<String> {
    paires(texte)
        .into_iter()
        .filter(|(c, _)| c.eq_ignore_ascii_case(cle))
        .map(|(_, v)| v)
        .collect()
}

enum Jeton {
    Texte(String),
    Ouvre,
    Ferme,
}

/// Le format KeyValues de Valve, réduit à ce qu'on lit : les paires
/// `"clé" "valeur"`, à plat. Les sections (`"clé" { … }`) sont traversées
/// sans être retenues — leurs paires internes remontent comme les autres.
fn paires(texte: &str) -> Vec<(String, String)> {
    let jetons = jetons(texte);
    let mut paires = Vec::new();
    let mut i = 0;
    while i < jetons.len() {
        match (&jetons[i], jetons.get(i + 1)) {
            (Jeton::Texte(cle), Some(Jeton::Texte(valeur))) => {
                paires.push((cle.clone(), valeur.clone()));
                i += 2;
            }
            _ => i += 1,
        }
    }
    paires
}

fn jetons(texte: &str) -> Vec<Jeton> {
    let mut jetons = Vec::new();
    let mut chars = texte.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => jetons.push(Jeton::Ouvre),
            '}' => jetons.push(Jeton::Ferme),
            '/' if chars.peek() == Some(&'/') => {
                // Commentaire jusqu'à la fin de la ligne.
                for suite in chars.by_ref() {
                    if suite == '\n' {
                        break;
                    }
                }
            }
            '"' => {
                let mut mot = String::new();
                while let Some(d) = chars.next() {
                    match d {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => mot.push('\n'),
                            Some('t') => mot.push('\t'),
                            Some(autre) => mot.push(autre),
                            None => break,
                        },
                        _ => mot.push(d),
                    }
                }
                jetons.push(Jeton::Texte(mot));
            }
            _ => {}
        }
    }
    jetons
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIBLIOTHEQUES: &str = r#""libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"apps"
		{
			"228980"		"0"
			"1086940"		"123"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"apps"
		{
			"892970"		"456"
		}
	}
}"#;

    fn manifeste(appid: u32, nom: &str, drapeaux: u32) -> String {
        format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\t\"{appid}\"\n\t\"Universe\"\t\t\"1\"\n\t\"name\"\t\t\"{nom}\"\n\t\"StateFlags\"\t\t\"{drapeaux}\"\n\t\"installdir\"\t\t\"X\"\n\t\"InstalledDepots\"\n\t{{\n\t\t\"1086941\"\n\t\t{{\n\t\t\t\"manifest\"\t\t\"42\"\n\t\t}}\n\t}}\n}}\n"
        )
    }

    #[test]
    fn lit_les_chemins_des_bibliotheques() {
        assert_eq!(
            valeurs(BIBLIOTHEQUES, "path"),
            vec!["C:\\Program Files (x86)\\Steam", "D:\\SteamLibrary"]
        );
    }

    #[test]
    fn lit_un_manifeste() {
        assert_eq!(
            lire_manifeste(&manifeste(1086940, "Baldur's Gate 3", 4)),
            Some(JeuInstalle { appid: 1086940, title: "Baldur's Gate 3".into() })
        );
    }

    #[test]
    fn ignore_un_jeu_pas_fini_d_installer() {
        assert_eq!(lire_manifeste(&manifeste(1086940, "Baldur's Gate 3", 1026)), None);
        assert!(lire_manifeste(&manifeste(1086940, "Baldur's Gate 3", 1030)).is_some());
    }

    #[test]
    fn ignore_ce_qui_n_est_pas_un_jeu() {
        assert_eq!(lire_manifeste(&manifeste(228980, "Steamworks Common Redistributables", 4)), None);
    }

    #[test]
    fn guillemets_echappes_dans_un_nom() {
        let texte = "\"AppState\" { \"appid\" \"10\" \"name\" \"Le \\\"vrai\\\" jeu\" }";
        assert_eq!(lire_manifeste(texte).unwrap().title, "Le \"vrai\" jeu");
    }

    #[test]
    fn meme_dossier_ecrit_differemment() {
        assert!(meme_dossier(Path::new("C:\\Program Files (x86)\\Steam"), Path::new("c:/program files (x86)/steam/")));
    }

    #[test]
    fn parcourt_une_installation_complete() {
        let racine = std::env::temp_dir().join(format!("squad-steam-test-{}", std::process::id()));
        let secondaire = racine.join("autre-disque");
        fs::create_dir_all(racine.join("steamapps")).unwrap();
        fs::create_dir_all(secondaire.join("steamapps")).unwrap();
        let vdf = format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} }}",
            racine.display(),
            secondaire.display()
        );
        fs::write(racine.join("steamapps/libraryfolders.vdf"), vdf).unwrap();
        fs::write(racine.join("steamapps/appmanifest_1086940.acf"), manifeste(1086940, "Baldur's Gate 3", 4)).unwrap();
        fs::write(racine.join("steamapps/appmanifest_228980.acf"), manifeste(228980, "Steamworks Common Redistributables", 4)).unwrap();
        fs::write(secondaire.join("steamapps/appmanifest_892970.acf"), manifeste(892970, "Valheim", 4)).unwrap();
        fs::write(secondaire.join("steamapps/appmanifest_1086940.acf"), manifeste(1086940, "Baldur's Gate 3", 4)).unwrap();

        let jeux = jeux_dans(&racine);
        fs::remove_dir_all(&racine).ok();
        assert_eq!(
            jeux.iter().map(|(j, _)| j.clone()).collect::<Vec<_>>(),
            vec![
                JeuInstalle { appid: 1086940, title: "Baldur's Gate 3".into() },
                JeuInstalle { appid: 892970, title: "Valheim".into() },
            ]
        );
        assert_eq!(jeux[1].1, secondaire.join("steamapps").join("common").join("X"));
    }
}
