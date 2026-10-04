// Sans cette ligne, une fenêtre de console noire s'ouvrirait à côté de l'app
// en version finale.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    squad_desktop_lib::run()
}
