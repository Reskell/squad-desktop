//! SQUAD — l'app de bureau.
//!
//! La fenêtre affiche le site SQUAD//LOG en ligne : le site reste le
//! cerveau, l'app ne réaffiche jamais ses propres versions des pages. Elle
//! ajoute seulement ce qui demande la machine :
//! - les jeux Steam installés (steam.rs), que le site propose d'importer ;
//! - les parties jouées (parties.rs), que le site enregistre — ce qui
//!   marque aussi la présence aux soirées ;
//! - les jeux hors Steam (hors_steam.rs) : un programme choisi par la
//!   personne, que l'app lance et reconnaît ;
//! - le mode invisible (reglages.rs) : la bande ne voit plus à quoi on joue ;
//! - le compagnon LoL (lol.rs) : une seconde fenêtre, ouverte avec le client
//!   LoL, qui lit la sélection des champions et la partie en cours ;
//! - une icône dans la barre des tâches, le lancement avec Windows, et les
//!   liens externes renvoyés vers le navigateur (liens.rs) ;
//! - les mises à jour, téléchargées et installées toutes seules depuis les
//!   versions publiées sur GitHub.
//!
//! Le même code donne aussi « SQUAD Compagnon », le compagnon LoL seul
//! (compilé avec SQUAD_COMPAGNON_SEUL, voir tauri.compagnon.conf.json) :
//! pas de fenêtre SQUAD, seulement celle du compagnon et l'icône.

mod hors_steam;
mod liens;
mod lol;
mod parties;
mod reglages;
mod steam;

use std::path::Path;
use std::time::Duration;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    webview::NewWindowResponse,
    AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_updater::UpdaterExt;

/// Un relevé des programmes lancés toutes les 30 secondes : assez fin pour
/// la règle des 15 minutes de présence, invisible pour le processeur.
const INTERVALLE: Duration = Duration::from_secs(30);
/// Relire la liste des jeux installés toutes les 20 relevés (10 minutes),
/// pour reconnaître un jeu installé pendant que l'app tourne.
const RELIRE_LES_JEUX_TOUS_LES: u32 = 20;
/// Passé par Windows quand l'app démarre avec la session : elle se lance
/// alors cachée dans la barre des tâches, sans ouvrir de fenêtre.
const ARG_DEMARRAGE: &str = "--au-demarrage";
/// Première recherche de mise à jour peu après le démarrage, puis toutes
/// les six heures : l'app reste souvent ouverte des jours entiers.
const PREMIERE_RECHERCHE: Duration = Duration::from_secs(60);
const ENTRE_DEUX_RECHERCHES: Duration = Duration::from_secs(6 * 60 * 60);
/// Vrai pour « SQUAD Compagnon », le compagnon LoL téléchargé seul : la
/// publication le compile avec la variable SQUAD_COMPAGNON_SEUL.
const SEUL: bool = option_env!("SQUAD_COMPAGNON_SEUL").is_some();
/// Le nom affiché dans les messages : celui de l'app installée.
const NOM: &str = if SEUL { "SQUAD Compagnon" } else { "SQUAD" };

/// Ramène la fenêtre au premier plan, qu'elle soit cachée ou réduite.
fn montrer(app: &AppHandle) {
    if SEUL {
        let _ = creer_le_compagnon(app);
        return;
    }
    if let Some(fenetre) = app.get_webview_window("main") {
        let _ = fenetre.show();
        let _ = fenetre.unminimize();
        let _ = fenetre.set_focus();
    }
}

/// Ouvre une adresse dans le navigateur habituel de la personne.
fn ouvrir_dans_le_navigateur(url: &Url) {
    let _ = tauri_plugin_opener::open_url(url.as_str(), None::<&str>);
}

fn reste_dans_la_fenetre(url: &Url) -> bool {
    liens::reste_dans_la_fenetre(url.scheme(), url.host_str().unwrap_or(""), url.path())
}

/// Les jeux Steam installés sur ce PC. L'app ne fait que lire : c'est le
/// site qui montre la liste, fait valider, et écrit dans la base avec le
/// compte de la personne — l'app n'a jamais besoin de sa session.
#[tauri::command]
async fn jeux_installes() -> Vec<steam::JeuInstalle> {
    steam::jeux_installes()
}

/// Les parties finies que le site n'a pas encore enregistrées.
#[tauri::command]
fn parties_a_envoyer(carnet: tauri::State<'_, parties::Parties>) -> Vec<parties::Partie> {
    carnet.a_envoyer()
}

/// Le site confirme avoir enregistré ces parties : l'app les oublie.
#[tauri::command]
fn parties_envoyees(carnet: tauri::State<'_, parties::Parties>, ids: Vec<String>) {
    carnet.envoyees(&ids)
}

/// Les jeux qui tournent en ce moment : le site les annonce à la bande
/// (« en jeu »), sauf en mode invisible.
#[tauri::command]
fn parties_en_cours(carnet: tauri::State<'_, parties::Parties>) -> Vec<parties::Partie> {
    carnet.en_cours()
}

/// Le mode invisible est-il coché dans le menu de l'icône ?
#[tauri::command]
fn presence_invisible(reglages: tauri::State<'_, reglages::Reglages>) -> bool {
    reglages.invisible()
}

/// Le compagnon LoL : la phase du client, la sélection des champions, la
/// partie en cours. Le client vient peut-être d'ouvrir : on relit les
/// programmes au besoin (au plus toutes les 5 secondes).
#[tauri::command]
async fn lol_etat(app: AppHandle) -> lol::EtatLol {
    let suivi = app.state::<lol::Lol>();
    if suivi.scan_du() {
        let mut systeme = sysinfo::System::new();
        let programmes = parties::programmes(&mut systeme);
        suivi.noter(lol::acces_client(&programmes));
    }
    suivi.etat().await
}

/// Importe une page de runes dans le client LoL (bouton du compagnon).
#[tauri::command]
async fn lol_importer_runes(app: AppHandle, page: lol::PageDeRunes) -> Result<(), String> {
    app.state::<lol::Lol>().importer_runes(&page).await
}

/// Choisit les sorts d'invocateur pendant la sélection (bouton du compagnon).
#[tauri::command]
async fn lol_importer_sorts(app: AppHandle, premier: u32, second: u32) -> Result<(), String> {
    app.state::<lol::Lol>().importer_sorts(premier, second).await
}

/// Range un set d'objets « SQUAD · … » dans le client LoL (bouton du compagnon).
#[tauri::command]
async fn lol_importer_objets(
    app: AppHandle,
    titre: String,
    champion: u32,
    blocs: Vec<lol::BlocObjets>,
) -> Result<(), String> {
    app.state::<lol::Lol>().importer_objets(&titre, champion, &blocs).await
}

/// Ouvre (ou ramène devant) la fenêtre du compagnon LoL.
#[tauri::command]
async fn ouvrir_compagnon(app: AppHandle) -> Result<(), String> {
    creer_le_compagnon(&app).map_err(|e| e.to_string())
}

/// Les jeux hors Steam associés sur ce PC (sans leur chemin complet).
#[tauri::command]
fn jeux_hors_steam(hs: tauri::State<'_, hors_steam::HorsSteam>) -> Vec<hors_steam::LienVisible> {
    hs.visibles()
}

/// Ouvre la fenêtre « Ouvrir » de Windows pour choisir le programme d'un
/// jeu. Rien si la personne annule.
#[tauri::command]
async fn choisir_programme(app: AppHandle) -> Option<hors_steam::Choix> {
    let fichier = app
        .dialog()
        .file()
        .set_title("Le programme qui lance le jeu")
        .add_filter("Programme", &["exe"])
        .blocking_pick_file()?;
    let chemin = fichier.into_path().ok()?;
    app.state::<hors_steam::HorsSteam>().retenir_choix(&chemin.to_string_lossy());
    Some(hors_steam::choix(&chemin))
}

/// Associe le programme choisi à la fiche d'un jeu.
#[tauri::command]
fn associer_jeu_hors_steam(
    hs: tauri::State<'_, hors_steam::HorsSteam>,
    game_id: String,
    title: String,
    chemin: String,
    instance_prism: Option<String>,
) -> Result<hors_steam::LienVisible, String> {
    hs.associer(hors_steam::Lien { game_id, title, chemin, instance_prism })
}

#[tauri::command]
fn retirer_jeu_hors_steam(hs: tauri::State<'_, hors_steam::HorsSteam>, game_id: String) {
    hs.retirer(&game_id)
}

/// Lance un jeu hors Steam associé, et le connecte à un serveur quand son
/// launcher sait le faire (Prism Launcher pour Minecraft).
#[tauri::command]
fn lancer_jeu_hors_steam(
    hs: tauri::State<'_, hors_steam::HorsSteam>,
    game_id: String,
    serveur: Option<String>,
) -> Result<(), String> {
    let lien = hs
        .trouver(&game_id)
        .ok_or_else(|| "Ce jeu n'est pas associé sur ce PC.".to_string())?;
    let chemin = Path::new(&lien.chemin);
    let mut commande = std::process::Command::new(chemin);
    commande.args(hors_steam::arguments(&lien, serveur.as_deref()));
    if let Some(dossier) = chemin.parent() {
        commande.current_dir(dossier);
    }
    commande
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Impossible de lancer {} : {e}", hors_steam::nom_du_programme(chemin)))
}

/// La boucle qui regarde quels jeux tournent, dans son propre fil pour ne
/// jamais bloquer la fenêtre.
fn demarrer_la_surveillance(app: &AppHandle) {
    let fichier = app.path().app_data_dir().ok().map(|d| d.join("parties.json"));
    app.manage(parties::Parties::charger(fichier));

    let app = app.clone();
    std::thread::spawn(move || {
        let mut systeme = sysinfo::System::new();
        let mut dossiers: Vec<(u32, String, String)> = Vec::new();
        let mut tour: u32 = 0;
        loop {
            if tour % RELIRE_LES_JEUX_TOUS_LES == 0 {
                dossiers = steam::jeux_et_dossiers()
                    .into_iter()
                    .map(|(jeu, dossier)| (jeu.appid, jeu.title, parties::normaliser(&dossier)))
                    .collect();
            }
            tour = tour.wrapping_add(1);

            let programmes = parties::programmes(&mut systeme);

            // Le client LoL : le compagnon s'ouvre avec lui et se ferme avec lui.
            let suivi = app.state::<lol::Lol>();
            let avant = suivi.acces().is_some();
            let acces = lol::acces_client(&programmes);
            let maintenant = acces.is_some();
            suivi.noter(acces);
            if maintenant && !avant && app.state::<reglages::Reglages>().compagnon_auto() {
                let _ = creer_le_compagnon(&app);
            } else if !maintenant && avant && !SEUL {
                // Seul, le compagnon EST l'app : il reste ouvert.
                if let Some(fenetre) = app.get_webview_window(COMPAGNON) {
                    let _ = fenetre.close();
                }
            }

            let mut vus = parties::reconnaitre(&programmes, &dossiers);
            vus.extend(hors_steam::reconnaitre(
                &app.state::<hors_steam::HorsSteam>().liens(),
                &programmes,
            ));
            let carnet = app.state::<parties::Parties>();
            let avant: Vec<String> = carnet.en_cours().into_iter().map(|p| p.id).collect();
            if carnet.relever(&vus, parties::maintenant_ms()) {
                // Le site écoute cet événement pour envoyer tout de suite ;
                // s'il ne l'entend pas, il repasse de lui-même chaque minute.
                let _ = app.emit("parties", ());
            }
            let apres: Vec<String> = carnet.en_cours().into_iter().map(|p| p.id).collect();
            if avant != apres {
                // Un jeu vient d'être lancé ou fermé : le site met à jour
                // « en jeu » pour la bande sans attendre.
                let _ = app.emit("en-cours", ());
            }
            std::thread::sleep(INTERVALLE);
        }
    });
}

/// Cherche une nouvelle version et l'installe sans rien demander. L'app se
/// ferme le temps de l'installation puis repart, d'où une règle : jamais
/// pendant qu'un jeu tourne, pour ne pas couper une partie en deux.
fn demarrer_les_mises_a_jour(app: &AppHandle) {
    // En développement (npm run dev), la version locale n'a pas de sens face
    // aux versions publiées : on ne cherche rien.
    if cfg!(debug_assertions) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(PREMIERE_RECHERCHE);
        loop {
            let un_jeu_tourne = !app.state::<parties::Parties>().en_cours().is_empty();
            if !un_jeu_tourne {
                tauri::async_runtime::block_on(async {
                    let Ok(updater) = app.updater() else { return };
                    if let Ok(Some(mise_a_jour)) = updater.check().await {
                        let _ = mise_a_jour.download_and_install(|_, _| {}, || {}).await;
                    }
                });
            }
            std::thread::sleep(ENTRE_DEUX_RECHERCHES);
        }
    });
}

/// Une petite fenêtre de message, sans bloquer l'app.
fn prevenir(app: &AppHandle, texte: String) {
    app.dialog().message(texte).title(NOM).show(|_| {});
}

/// « Vérifier les mises à jour », depuis le menu de l'icône : la même
/// recherche que la vérification automatique, mais tout de suite, et avec
/// une réponse dans les deux cas. L'installeur ferme l'app puis la relance.
fn verifier_maintenant(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let updater = match app.updater() {
            Ok(updater) => updater,
            Err(e) => return prevenir(&app, format!("Impossible de chercher une mise à jour : {e}")),
        };
        let trouvee = match tauri::async_runtime::block_on(updater.check()) {
            Ok(trouvee) => trouvee,
            Err(e) => return prevenir(&app, format!("Impossible de chercher une mise à jour : {e}")),
        };
        let Some(mise_a_jour) = trouvee else {
            return prevenir(&app, format!("{NOM} est à jour (version {}).", app.package_info().version));
        };
        prevenir(
            &app,
            format!("Version {} trouvée : installation, {NOM} va redémarrer.", mise_a_jour.version),
        );
        if let Err(e) = tauri::async_runtime::block_on(mise_a_jour.download_and_install(|_, _| {}, || {})) {
            prevenir(&app, format!("La mise à jour a échoué : {e}"));
        }
    });
}

/// L'étiquette de la fenêtre du compagnon LoL.
const COMPAGNON: &str = "compagnon";
const OVERLAY: &str = "overlay";

/// L'overlay en jeu : une fenêtre transparente, toujours devant, que la
/// souris traverse, posée sur tout l'écran principal. Elle affiche la page
/// /compagnon/lol/overlay du site (ce que le tableau des scores montre déjà,
/// les objectifs publics, ton matchup). Le jeu doit être en « plein écran
/// fenêtré » (le plein écran exclusif passe devant toute fenêtre).
fn ouvrir_l_overlay(app: &AppHandle) -> tauri::Result<()> {
    if app.get_webview_window(OVERLAY).is_some() {
        return Ok(());
    }
    let url = Url::parse(&format!("https://{}/compagnon/lol/overlay", liens::SITE)).expect("adresse de l'overlay");
    let mut fenetre = WebviewWindowBuilder::new(app, OVERLAY, WebviewUrl::External(url))
        .title("SQUAD Overlay")
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .on_navigation(reste_dans_la_fenetre);
    if let Ok(Some(ecran)) = app.primary_monitor() {
        let taille = ecran.size().to_logical::<f64>(ecran.scale_factor());
        fenetre = fenetre.inner_size(taille.width, taille.height).position(0.0, 0.0);
    }
    let fenetre = fenetre.build()?;
    // Les clics passent au jeu : l'overlay ne se touche pas.
    let _ = fenetre.set_ignore_cursor_events(true);
    Ok(())
}

/// Montre ou retire l'overlay en jeu (demandé par la page du compagnon).
#[tauri::command]
async fn lol_overlay(app: AppHandle, afficher: bool) -> Result<(), String> {
    if afficher {
        ouvrir_l_overlay(&app).map_err(|e| e.to_string())
    } else {
        if let Some(fenetre) = app.get_webview_window(OVERLAY) {
            let _ = fenetre.close();
        }
        Ok(())
    }
}

/// La fenêtre du compagnon LoL : étroite, à droite de l'écran, à côté du
/// client. Elle affiche la page /compagnon/lol du site, qui lit le client
/// par la commande `lol_etat`.
fn creer_le_compagnon(app: &AppHandle) -> tauri::Result<()> {
    if let Some(fenetre) = app.get_webview_window(COMPAGNON) {
        let _ = fenetre.show();
        let _ = fenetre.unminimize();
        let _ = fenetre.set_focus();
        return Ok(());
    }
    let url = Url::parse(&format!("https://{}/compagnon/lol", liens::SITE)).expect("adresse du compagnon");
    // Une vraie appli (menu à gauche, pages) : grande par défaut, sans
    // dépasser l'écran. Rétrécie, elle passe en colonne d'icônes.
    let ecran = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|e| e.size().to_logical::<f64>(e.scale_factor()));
    let (largeur, hauteur) = match ecran {
        Some(t) => ((t.width - 32.0).clamp(420.0, 1240.0), (t.height - 80.0).clamp(480.0, 820.0)),
        None => (1240.0, 820.0),
    };
    let mut fenetre = WebviewWindowBuilder::new(app, COMPAGNON, WebviewUrl::External(url))
        .title("SQUAD Compagnon")
        .inner_size(largeur, hauteur)
        .min_inner_size(400.0, 480.0)
        .on_navigation(|url| {
            if reste_dans_la_fenetre(url) {
                true
            } else {
                ouvrir_dans_le_navigateur(url);
                false
            }
        })
        .on_new_window(|url, _| {
            ouvrir_dans_le_navigateur(&url);
            NewWindowResponse::Deny
        });
    if let Some(t) = ecran {
        fenetre = fenetre.position((t.width - largeur - 16.0).max(0.0), 40.0);
    }
    fenetre.build()?;
    Ok(())
}

/// La fenêtre, construite ici plutôt que par la configuration pour pouvoir
/// décider où vont les liens. Cachée quand Windows lance l'app au démarrage.
fn creer_la_fenetre(app: &AppHandle, visible: bool) -> tauri::Result<()> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .cloned()
        .expect("la fenêtre « main » manque dans tauri.conf.json");

    WebviewWindowBuilder::from_config(app, &config)?
        .visible(visible)
        // Une page qui remplace celle de la fenêtre (lien normal) : hors du
        // site et de la connexion, elle part dans le navigateur.
        .on_navigation(|url| {
            if reste_dans_la_fenetre(url) {
                true
            } else {
                ouvrir_dans_le_navigateur(url);
                false
            }
        })
        // Un lien « nouvel onglet » : toujours le navigateur, jamais une
        // seconde fenêtre SQUAD sans barre d'adresse.
        .on_new_window(|url, _| {
            ouvrir_dans_le_navigateur(&url);
            NewWindowResponse::Deny
        })
        .build()?;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![ARG_DEMARRAGE]),
        ))
        .setup(|app| {
            let lance_par_windows = std::env::args().any(|a| a == ARG_DEMARRAGE);
            let donnees = app.path().app_data_dir().ok();
            app.manage(reglages::Reglages::charger(donnees.as_ref().map(|d| d.join("reglages.json"))));
            app.manage(hors_steam::HorsSteam::charger(donnees.as_ref().map(|d| d.join("hors-steam.json"))));
            app.manage(lol::Lol::nouveau());
            if SEUL {
                // Au démarrage de Windows, il attend LoL sans rien ouvrir.
                if !lance_par_windows {
                    creer_le_compagnon(app.handle())?;
                }
            } else {
                creer_la_fenetre(app.handle(), !lance_par_windows)?;
            }

            let ouvrir = MenuItem::with_id(app, "ouvrir", "Ouvrir SQUAD", true, None::<&str>)?;
            let demarrage = CheckMenuItem::with_id(
                app,
                "demarrage",
                "Lancer avec Windows",
                true,
                app.autolaunch().is_enabled().unwrap_or(false),
                None::<&str>,
            )?;
            // Invisible : l'app continue de compter le temps de jeu, mais le
            // site n'annonce plus à la bande à quoi on joue.
            let invisible = CheckMenuItem::with_id(
                app,
                "invisible",
                "Invisible pour la bande",
                true,
                app.state::<reglages::Reglages>().invisible(),
                None::<&str>,
            )?;
            let compagnon = MenuItem::with_id(app, "compagnon", "Ouvrir le compagnon LoL", true, None::<&str>)?;
            let compagnon_auto = CheckMenuItem::with_id(
                app,
                "compagnon_auto",
                "Compagnon LoL avec le jeu",
                true,
                app.state::<reglages::Reglages>().compagnon_auto(),
                None::<&str>,
            )?;
            let separateur = PredefinedMenuItem::separator(app)?;
            // La version installée, grisée : utile pour savoir si la mise à
            // jour est passée quand un pote signale un souci.
            let version = MenuItem::with_id(
                app,
                "version",
                format!("{NOM} {}", app.package_info().version),
                false,
                None::<&str>,
            )?;
            let verifier = MenuItem::with_id(app, "verifier", "Vérifier les mises à jour", true, None::<&str>)?;
            let quitter = MenuItem::with_id(app, "quitter", "Quitter", true, None::<&str>)?;
            // Seul, ni « Ouvrir SQUAD » ni « Invisible » : il n'y a ni site
            // complet ni présence à cacher.
            let menu = if SEUL {
                Menu::with_items(
                    app,
                    &[&compagnon, &demarrage, &compagnon_auto, &separateur, &version, &verifier, &quitter],
                )?
            } else {
                Menu::with_items(
                    app,
                    &[
                        &ouvrir,
                        &compagnon,
                        &demarrage,
                        &invisible,
                        &compagnon_auto,
                        &separateur,
                        &version,
                        &verifier,
                        &quitter,
                    ],
                )?
            };

            let case_demarrage = demarrage.clone();
            let case_invisible = invisible.clone();
            let case_compagnon = compagnon_auto.clone();
            TrayIconBuilder::with_id("squad")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip(NOM)
                .menu(&menu)
                // Clic gauche = ouvrir, clic droit = le menu : le réflexe
                // habituel des icônes de la barre des tâches sous Windows.
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "ouvrir" => montrer(app),
                    "demarrage" => {
                        // On repart de l'état réel plutôt que de la case :
                        // si Windows refuse, la case doit dire la vérité.
                        let actif = app.autolaunch().is_enabled().unwrap_or(false);
                        let _ = if actif {
                            app.autolaunch().disable()
                        } else {
                            app.autolaunch().enable()
                        };
                        let _ = case_demarrage.set_checked(app.autolaunch().is_enabled().unwrap_or(false));
                    }
                    "invisible" => {
                        let etat = app.state::<reglages::Reglages>().basculer_invisible();
                        let _ = case_invisible.set_checked(etat);
                        let _ = app.emit("invisible", etat);
                    }
                    "verifier" => verifier_maintenant(app),
                    "compagnon" => {
                        let _ = creer_le_compagnon(app);
                    }
                    "compagnon_auto" => {
                        let etat = app.state::<reglages::Reglages>().basculer_compagnon_auto();
                        let _ = case_compagnon.set_checked(etat);
                    }
                    "quitter" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|icone, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        montrer(icone.app_handle());
                    }
                })
                .build(app)?;

            demarrer_la_surveillance(app.handle());
            demarrer_les_mises_a_jour(app.handle());
            Ok(())
        })
        // Fermer la fenêtre la cache au lieu de quitter : l'app doit rester
        // en vie pour voir les jeux lancés. On quitte vraiment depuis le
        // menu de l'icône.
        .on_window_event(|fenetre, event| {
            // Seule la fenêtre principale se cache au lieu de fermer : le
            // compagnon, lui, se ferme pour de bon — sauf quand il est
            // l'app à lui tout seul.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if fenetre.label() == "main" || (SEUL && fenetre.label() == COMPAGNON) {
                    let _ = fenetre.hide();
                    api.prevent_close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            jeux_installes,
            parties_a_envoyer,
            parties_envoyees,
            parties_en_cours,
            presence_invisible,
            jeux_hors_steam,
            choisir_programme,
            associer_jeu_hors_steam,
            retirer_jeu_hors_steam,
            lancer_jeu_hors_steam,
            lol_etat,
            ouvrir_compagnon,
            lol_importer_runes,
            lol_importer_sorts,
            lol_importer_objets,
            lol_overlay
        ])
        .run(tauri::generate_context!())
        .expect("l'app n'a pas pu démarrer");
}
