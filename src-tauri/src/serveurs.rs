//! « Allumer » (SQUAD//JOIN) : démarrer le serveur de la bande hébergé sur
//! ce PC, quand quelqu'un le demande depuis le site.
//!
//! L'app ne fait qu'une chose : `docker start <conteneur>`. Le nom du
//! conteneur vient de la fiche du serveur (choisi par son auteur) et passe
//! par une règle stricte — lettres, chiffres, tiret, point, souligné — avant
//! d'être donné à Docker comme un seul argument : jamais de ligne de commande
//! fabriquée à partir de texte, rien qui puisse lancer autre chose.

use std::process::Command;

/// Le nom d'un conteneur Docker tel qu'on l'accepte.
pub fn nom_valide(conteneur: &str) -> bool {
    let mut caracteres = conteneur.chars();
    match caracteres.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    conteneur.len() <= 64 && caracteres.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Démarre le conteneur. Rend un message lisible en cas d'échec.
pub fn allumer(conteneur: &str) -> Result<(), String> {
    if !nom_valide(conteneur) {
        return Err("Nom de conteneur refusé.".into());
    }
    let mut commande = Command::new("docker");
    commande.arg("start").arg(conteneur);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Pas de fenêtre noire qui clignote chez l'hôte.
        commande.creation_flags(0x0800_0000);
    }
    let sortie = commande
        .output()
        .map_err(|_| "Docker est introuvable sur le PC de l'hôte.".to_string())?;
    if sortie.status.success() {
        return Ok(());
    }
    let erreur = String::from_utf8_lossy(&sortie.stderr).trim().to_string();
    if erreur.contains("error during connect") || erreur.contains("Cannot connect") || erreur.contains("pipe") {
        return Err("Docker ne répond pas chez l'hôte (Docker Desktop est-il lancé ?).".into());
    }
    if erreur.contains("No such container") {
        return Err(format!("Aucun conteneur « {conteneur} » chez l'hôte."));
    }
    Err(format!("Docker a refusé : {}", erreur.chars().take(200).collect::<String>()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noms() {
        assert!(nom_valide("minecraft"));
        assert!(nom_valide("mc-survie_1.21"));
        assert!(!nom_valide(""));
        assert!(!nom_valide("-x"));
        assert!(!nom_valide("a b"));
        assert!(!nom_valide("a;rm -rf"));
        assert!(!nom_valide(&"a".repeat(65)));
    }
}
