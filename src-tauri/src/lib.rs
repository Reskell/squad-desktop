//! SQUAD — l'app de bureau.
//!
//! Étape 1 : une fenêtre qui affiche le site SQUAD//LOG en ligne, et une
//! icône dans la barre des tâches.
//! Étape 3 : la commande `jeux_installes`, que le site appelle pour proposer
//! les jeux Steam installés sur ce PC. Le site reste le cerveau : l'app ne
//! réaffiche jamais ses propres versions des pages, elle ajoutera seulement
//! ce qui demande la machine (détection des jeux, présence, lancement).

mod steam;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WindowEvent,
};

/// Ramène la fenêtre au premier plan, qu'elle soit cachée ou réduite.
fn montrer(app: &AppHandle) {
    if let Some(fenetre) = app.get_webview_window("main") {
        let _ = fenetre.show();
        let _ = fenetre.unminimize();
        let _ = fenetre.set_focus();
    }
}

/// Les jeux Steam installés sur ce PC. L'app ne fait que lire : c'est le
/// site qui montre la liste, fait valider, et écrit dans la base avec le
/// compte de la personne — l'app n'a jamais besoin de sa session.
#[tauri::command]
async fn jeux_installes() -> Vec<steam::JeuInstalle> {
    steam::jeux_installes()
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let ouvrir = MenuItem::with_id(app, "ouvrir", "Ouvrir SQUAD", true, None::<&str>)?;
            let quitter = MenuItem::with_id(app, "quitter", "Quitter", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&ouvrir, &quitter])?;

            TrayIconBuilder::with_id("squad")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("SQUAD")
                .menu(&menu)
                // Clic gauche = ouvrir, clic droit = le menu : le réflexe
                // habituel des icônes de la barre des tâches sous Windows.
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "ouvrir" => montrer(app),
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
            Ok(())
        })
        // Fermer la fenêtre la cache au lieu de quitter : l'app doit rester
        // en vie pour voir les jeux lancés (étape 4). On quitte vraiment
        // depuis le menu de l'icône.
        .on_window_event(|fenetre, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = fenetre.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![jeux_installes])
        .run(tauri::generate_context!())
        .expect("SQUAD n'a pas pu démarrer");
}
