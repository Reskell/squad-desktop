//! SQUAD — l'app de bureau.
//!
//! La fenêtre affiche le site SQUAD//LOG en ligne : le site reste le
//! cerveau, l'app ne réaffiche jamais ses propres versions des pages. Elle
//! ajoute seulement ce qui demande la machine :
//! - les jeux Steam installés (steam.rs), que le site propose d'importer ;
//! - les parties jouées (parties.rs), que le site enregistre — ce qui
//!   marque aussi la présence aux soirées ;
//! - une icône dans la barre des tâches, le lancement avec Windows, et les
//!   liens externes renvoyés vers le navigateur (liens.rs).

mod liens;
mod parties;
mod steam;

use std::time::Duration;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    webview::NewWindowResponse,
    AppHandle, Emitter, Manager, Url, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

/// Un relevé des programmes lancés toutes les 30 secondes : assez fin pour
/// la règle des 15 minutes de présence, invisible pour le processeur.
const INTERVALLE: Duration = Duration::from_secs(30);
/// Relire la liste des jeux installés toutes les 20 relevés (10 minutes),
/// pour reconnaître un jeu installé pendant que l'app tourne.
const RELIRE_LES_JEUX_TOUS_LES: u32 = 20;
/// Passé par Windows quand l'app démarre avec la session : elle se lance
/// alors cachée dans la barre des tâches, sans ouvrir de fenêtre.
const ARG_DEMARRAGE: &str = "--au-demarrage";

/// Ramène la fenêtre au premier plan, qu'elle soit cachée ou réduite.
fn montrer(app: &AppHandle) {
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

/// Les jeux qui tournent en ce moment (pour un futur « en jeu » sur le site).
#[tauri::command]
fn parties_en_cours(carnet: tauri::State<'_, parties::Parties>) -> Vec<parties::Partie> {
    carnet.en_cours()
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

            let vus = parties::jeux_qui_tournent(&mut systeme, &dossiers);
            let carnet = app.state::<parties::Parties>();
            if carnet.relever(&vus, parties::maintenant_ms()) {
                // Le site écoute cet événement pour envoyer tout de suite ;
                // s'il ne l'entend pas, il repasse de lui-même chaque minute.
                let _ = app.emit("parties", ());
            }
            std::thread::sleep(INTERVALLE);
        }
    });
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
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![ARG_DEMARRAGE]),
        ))
        .setup(|app| {
            let lance_par_windows = std::env::args().any(|a| a == ARG_DEMARRAGE);
            creer_la_fenetre(app.handle(), !lance_par_windows)?;

            let ouvrir = MenuItem::with_id(app, "ouvrir", "Ouvrir SQUAD", true, None::<&str>)?;
            let demarrage = CheckMenuItem::with_id(
                app,
                "demarrage",
                "Lancer avec Windows",
                true,
                app.autolaunch().is_enabled().unwrap_or(false),
                None::<&str>,
            )?;
            let separateur = PredefinedMenuItem::separator(app)?;
            let quitter = MenuItem::with_id(app, "quitter", "Quitter", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&ouvrir, &demarrage, &separateur, &quitter])?;

            let case_demarrage = demarrage.clone();
            TrayIconBuilder::with_id("squad")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("SQUAD")
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
            Ok(())
        })
        // Fermer la fenêtre la cache au lieu de quitter : l'app doit rester
        // en vie pour voir les jeux lancés. On quitte vraiment depuis le
        // menu de l'icône.
        .on_window_event(|fenetre, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = fenetre.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            jeux_installes,
            parties_a_envoyer,
            parties_envoyees,
            parties_en_cours
        ])
        .run(tauri::generate_context!())
        .expect("SQUAD n'a pas pu démarrer");
}
