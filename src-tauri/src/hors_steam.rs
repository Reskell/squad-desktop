//! Les jeux hors Steam : Minecraft, un jeu d'un autre launcher, un jeu
//! téléchargé à part.
//!
//! La personne choisit elle-même le programme à lancer (fenêtre « Ouvrir »
//! de Windows) et l'associe à la fiche du jeu sur le site. L'association
//! reste sur ce PC, dans `hors-steam.json` : le chemin ne quitte jamais
//! l'app, le site ne connaît que la fiche.
//!
//! Sécurité : le site ne peut pas faire lancer n'importe quoi. Il ne peut
//! associer qu'un programme que la personne vient de choisir dans la fenêtre
//! « Ouvrir », et ne lance ensuite qu'un jeu associé, par sa fiche.

use crate::parties::{normaliser, Programme, Vu};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lien {
    /// La fiche du jeu sur le site.
    pub game_id: String,
    pub title: String,
    /// Le programme à lancer.
    pub chemin: String,
    /// Pour Prism Launcher : l'instance Minecraft à lancer.
    #[serde(default)]
    pub instance_prism: Option<String>,
}

/// Ce que le site voit d'un lien : jamais le chemin complet, seulement le
/// nom du programme pour que la personne s'y retrouve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LienVisible {
    pub game_id: String,
    pub title: String,
    pub programme: String,
    pub instance_prism: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstancePrism {
    pub id: String,
    pub nom: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Choix {
    pub chemin: String,
    pub programme: String,
    pub prism: bool,
    pub instances: Vec<InstancePrism>,
}

pub struct HorsSteam {
    liens: Mutex<Vec<Lien>>,
    /// Les programmes choisis dans la fenêtre « Ouvrir » pendant cette
    /// session : les seuls que le site a le droit d'associer.
    choisis: Mutex<Vec<String>>,
    fichier: Option<PathBuf>,
}

impl HorsSteam {
    pub fn charger(fichier: Option<PathBuf>) -> Self {
        let liens: Vec<Lien> = fichier
            .as_deref()
            .and_then(|f| fs::read_to_string(f).ok())
            .and_then(|texte| serde_json::from_str(&texte).ok())
            .unwrap_or_default();
        HorsSteam {
            liens: Mutex::new(liens),
            choisis: Mutex::new(Vec::new()),
            fichier,
        }
    }

    pub fn liens(&self) -> Vec<Lien> {
        self.liens.lock().unwrap().clone()
    }

    pub fn visibles(&self) -> Vec<LienVisible> {
        self.liens().iter().map(visible).collect()
    }

    pub fn trouver(&self, game_id: &str) -> Option<Lien> {
        self.liens().into_iter().find(|l| l.game_id == game_id)
    }

    /// Retenir un programme choisi dans la fenêtre « Ouvrir ».
    pub fn retenir_choix(&self, chemin: &str) {
        let mut choisis = self.choisis.lock().unwrap();
        if !choisis.iter().any(|c| c == chemin) {
            choisis.push(chemin.to_string());
        }
    }

    /// Associer un programme à une fiche. Refusé si le programme n'a pas été
    /// choisi par la personne, ou si l'instance Prism n'existe pas.
    pub fn associer(&self, lien: Lien) -> Result<LienVisible, String> {
        if !self.choisis.lock().unwrap().iter().any(|c| *c == lien.chemin) {
            return Err("Choisis d'abord le programme avec le bouton « Choisir ».".into());
        }
        if lien.game_id.trim().is_empty() || lien.title.trim().is_empty() {
            return Err("Il manque le jeu.".into());
        }
        if let Some(instance) = &lien.instance_prism {
            if !instances_prism(Path::new(&lien.chemin)).iter().any(|i| &i.id == instance) {
                return Err("Cette instance Prism est introuvable.".into());
            }
        }
        let rendu = visible(&lien);
        {
            let mut liens = self.liens.lock().unwrap();
            liens.retain(|l| l.game_id != lien.game_id);
            liens.push(lien);
        }
        self.ecrire();
        Ok(rendu)
    }

    pub fn retirer(&self, game_id: &str) {
        self.liens.lock().unwrap().retain(|l| l.game_id != game_id);
        self.ecrire();
    }

    fn ecrire(&self) {
        let Some(fichier) = &self.fichier else { return };
        let Ok(texte) = serde_json::to_string_pretty(&*self.liens.lock().unwrap()) else {
            return;
        };
        if let Some(dossier) = fichier.parent() {
            let _ = fs::create_dir_all(dossier);
        }
        let provisoire = fichier.with_extension("json.tmp");
        if fs::write(&provisoire, texte).is_ok() {
            let _ = fs::rename(&provisoire, fichier);
        }
    }
}

fn visible(lien: &Lien) -> LienVisible {
    LienVisible {
        game_id: lien.game_id.clone(),
        title: lien.title.clone(),
        programme: nom_du_programme(Path::new(&lien.chemin)),
        instance_prism: lien.instance_prism.clone(),
    }
}

/// Le nom du programme (`Celeste.exe`). Découpé à la main sur les deux
/// séparateurs, pour se comporter pareil partout (et dans les tests).
pub fn nom_du_programme(chemin: &Path) -> String {
    chemin
        .to_string_lossy()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Le dossier qui contient le programme, normalisé.
fn dossier_du_programme(chemin: &str) -> String {
    match chemin.rfind(['/', '\\']) {
        Some(i) => normaliser(Path::new(&chemin[..i])),
        None => String::new(),
    }
}

pub fn est_prism(chemin: &Path) -> bool {
    nom_du_programme(chemin).to_lowercase().starts_with("prismlauncher")
}

/// Ce qu'on montre au site après la fenêtre « Ouvrir ».
pub fn choix(chemin: &Path) -> Choix {
    let prism = est_prism(chemin);
    Choix {
        chemin: chemin.to_string_lossy().to_string(),
        programme: nom_du_programme(chemin),
        prism,
        instances: if prism { instances_prism(chemin) } else { Vec::new() },
    }
}

/// Les instances Minecraft de Prism Launcher : dans le dossier de Prism
/// (version portable) ou dans `%APPDATA%\PrismLauncher`.
pub fn instances_prism(chemin: &Path) -> Vec<InstancePrism> {
    let mut dossiers: Vec<PathBuf> = Vec::new();
    if let Some(parent) = chemin.parent() {
        dossiers.push(parent.join("instances"));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dossiers.push(PathBuf::from(appdata).join("PrismLauncher").join("instances"));
    }
    let mut instances: Vec<InstancePrism> = Vec::new();
    for dossier in dossiers {
        let Ok(entrees) = fs::read_dir(&dossier) else { continue };
        for entree in entrees.flatten() {
            let id = entree.file_name().to_string_lossy().to_string();
            if id.starts_with('.') || id.starts_with('_') || instances.iter().any(|i| i.id == id) {
                continue;
            }
            let Ok(cfg) = fs::read_to_string(entree.path().join("instance.cfg")) else {
                continue;
            };
            let nom = cfg
                .lines()
                .find_map(|l| l.strip_prefix("name="))
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| id.clone());
            instances.push(InstancePrism { id, nom });
        }
    }
    instances.sort_by(|a, b| a.nom.to_lowercase().cmp(&b.nom.to_lowercase()));
    instances
}

/// Le client Riot (`RiotClientServices.exe`) lance tous les jeux Riot et
/// tourne en permanence : on ne peut ni le lancer « tout court », ni
/// conclure qu'on joue parce qu'il est ouvert. Pour chaque jeu Riot : le
/// produit à demander au client, et le programme du jeu lui-même.
const JEUX_RIOT: &[(&str, &str, &str)] = &[
    // (mot du titre, produit Riot, début du nom du programme du jeu)
    ("league", "league_of_legends", "league"), // client LoL (salon, sélection) et partie
    ("valorant", "valorant", "valorant-win64-shipping"),
    ("runeterra", "bacon", "lor"),
];

pub fn est_riot(chemin: &Path) -> bool {
    nom_du_programme(chemin).to_lowercase().starts_with("riotclientservices")
}

fn jeu_riot(lien: &Lien) -> Option<&'static (&'static str, &'static str, &'static str)> {
    if !est_riot(Path::new(&lien.chemin)) {
        return None;
    }
    let titre = lien.title.to_lowercase();
    JEUX_RIOT.iter().find(|(mot, _, _)| titre.contains(mot))
}

/// Une adresse de serveur acceptable : `hote` ou `hote:port`, sans espace
/// ni caractère qui pourrait passer pour une option.
pub fn adresse_valide(adresse: &str) -> bool {
    !adresse.is_empty()
        && adresse.len() <= 255
        && !adresse.starts_with('-')
        && adresse.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_'))
}

/// Les arguments à passer au programme. Prism sait lancer une instance et
/// se connecter directement à un serveur.
pub fn arguments(lien: &Lien, serveur: Option<&str>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some((_, produit, _)) = jeu_riot(lien) {
        args.push(format!("--launch-product={produit}"));
        args.push("--launch-patchline=live".to_string());
        return args;
    }
    if est_prism(Path::new(&lien.chemin)) {
        if let Some(instance) = &lien.instance_prism {
            args.push("--launch".to_string());
            args.push(instance.clone());
            if let Some(serveur) = serveur.filter(|s| adresse_valide(s)) {
                args.push("--server".to_string());
                args.push(serveur.to_string());
            }
        }
    }
    args
}

/// Un lien qui lance Minecraft (par Prism ou un autre launcher) : le jeu
/// tourne dans Java, pas dans le dossier du launcher.
fn est_minecraft(lien: &Lien) -> bool {
    est_prism(Path::new(&lien.chemin)) || lien.title.to_lowercase().contains("minecraft")
}

/// Le lien tourne-t-il ? Minecraft : un Java lancé pour Minecraft. Les
/// autres : un programme lancé depuis le dossier du programme choisi — ou
/// le programme lui-même si ce dossier est trop général (`C:\`, `C:\Jeux`)
/// pour désigner un seul jeu.
pub fn tourne(lien: &Lien, programmes: &[Programme]) -> bool {
    if est_riot(Path::new(&lien.chemin)) {
        // Le client Riot ouvert ne veut rien dire : seul le jeu compte.
        return match jeu_riot(lien) {
            Some((_, _, programme)) => programmes.iter().any(|p| p.nom.starts_with(programme)),
            None => false,
        };
    }
    if est_minecraft(lien) {
        return programmes.iter().any(|p| {
            p.nom.starts_with("java")
                && (p.commande.contains("minecraft") || p.commande.contains("prismlauncher"))
        });
    }
    let exe = normaliser(Path::new(&lien.chemin));
    let dossier = dossier_du_programme(&lien.chemin);
    let profondeur = dossier.trim_end_matches('/').split('/').filter(|s| !s.is_empty()).count();
    if profondeur >= 3 {
        programmes.iter().any(|p| !p.exe.is_empty() && p.exe.starts_with(&dossier))
    } else {
        programmes.iter().any(|p| p.exe == exe)
    }
}

/// Les jeux hors Steam qui tournent.
pub fn reconnaitre(liens: &[Lien], programmes: &[Programme]) -> Vec<Vu> {
    liens
        .iter()
        .filter(|l| tourne(l, programmes))
        .map(|l| Vu::hors_steam(l.game_id.clone(), l.title.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lien(chemin: &str, title: &str) -> Lien {
        Lien {
            game_id: "fiche".into(),
            title: title.into(),
            chemin: chemin.into(),
            instance_prism: None,
        }
    }

    fn exe(chemin: &str) -> Programme {
        Programme {
            exe: normaliser(Path::new(chemin)),
            nom: nom_du_programme(Path::new(chemin)).to_lowercase(),
            commande: String::new(),
        }
    }

    #[test]
    fn un_jeu_dans_son_dossier() {
        let l = lien("D:\\Jeux\\Celeste\\Celeste.exe", "Celeste");
        assert!(tourne(&l, &[exe("D:\\Jeux\\Celeste\\bin\\Celeste.bin.exe")]));
        assert!(!tourne(&l, &[exe("D:\\Jeux\\CelesteMod\\x.exe")]));
    }

    #[test]
    fn un_dossier_trop_general_demande_le_programme_lui_meme() {
        let l = lien("D:\\Jeux\\jeu.exe", "Jeu");
        assert!(!tourne(&l, &[exe("D:\\Jeux\\autre.exe")]));
        assert!(tourne(&l, &[exe("D:\\Jeux\\jeu.exe")]));
    }

    #[test]
    fn minecraft_par_prism_se_voit_dans_java() {
        let mut l = lien("C:\\Users\\x\\AppData\\Local\\Programs\\PrismLauncher\\prismlauncher.exe", "Minecraft");
        l.instance_prism = Some("1.21".into());
        let prism_seul = exe("C:\\Users\\x\\AppData\\Local\\Programs\\PrismLauncher\\prismlauncher.exe");
        // Le launcher ouvert ne veut pas dire qu'on joue.
        assert!(!tourne(&l, &[prism_seul.clone()]));
        let java = Programme {
            exe: normaliser(Path::new("C:\\Program Files\\Java\\bin\\javaw.exe")),
            nom: "javaw.exe".into(),
            commande: "javaw -djava.library.path=c:/users/x/appdata/roaming/prismlauncher/instances/1.21/natives".into(),
        };
        assert!(tourne(&l, &[prism_seul, java]));
    }

    #[test]
    fn prism_lance_l_instance_et_le_serveur() {
        let mut l = lien("C:\\Prism\\prismlauncher.exe", "Minecraft");
        l.instance_prism = Some("Survie".into());
        assert_eq!(arguments(&l, None), vec!["--launch", "Survie"]);
        assert_eq!(
            arguments(&l, Some("mc.example.fr:25565")),
            vec!["--launch", "Survie", "--server", "mc.example.fr:25565"]
        );
        // Une adresse douteuse est ignorée, jamais passée au programme.
        assert_eq!(arguments(&l, Some("--help")), vec!["--launch", "Survie"]);
        assert!(arguments(&lien("D:\\Jeux\\Celeste\\Celeste.exe", "Celeste"), Some("x:1")).is_empty());
    }

    #[test]
    fn league_passe_par_le_client_riot() {
        let l = lien("C:\\Riot Games\\Riot Client\\RiotClientServices.exe", "League of Legends");
        assert_eq!(
            arguments(&l, Some("x:1")),
            vec!["--launch-product=league_of_legends", "--launch-patchline=live"]
        );
        // Le client Riot ouvert : pas une partie.
        let client = exe("C:\\Riot Games\\Riot Client\\RiotClientServices.exe");
        assert!(!tourne(&l, &[client.clone()]));
        let jeu = exe("C:\\Riot Games\\League of Legends\\Game\\League of Legends.exe");
        assert!(tourne(&l, &[client, jeu]));
    }

    #[test]
    fn on_n_associe_que_ce_qu_on_a_choisi() {
        let hs = HorsSteam::charger(None);
        let l = lien("D:\\Jeux\\Celeste\\Celeste.exe", "Celeste");
        assert!(hs.associer(l.clone()).is_err());
        hs.retenir_choix("D:\\Jeux\\Celeste\\Celeste.exe");
        let vu = hs.associer(l).unwrap();
        assert_eq!(vu.programme, "Celeste.exe");
        assert_eq!(hs.liens().len(), 1);
        hs.retirer("fiche");
        assert!(hs.liens().is_empty());
    }
}
