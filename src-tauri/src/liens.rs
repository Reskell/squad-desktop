//! Ce qui s'ouvre dans la fenêtre SQUAD, et ce qui part dans le navigateur.
//!
//! La fenêtre est faite pour le site. Une fiche Steam, une vidéo, une
//! invitation Discord suivie depuis le site s'ouvrent dans le navigateur
//! habituel — sinon on se retrouve à naviguer sur YouTube dans l'app, sans
//! barre d'adresse ni bouton retour. Seules restent dans la fenêtre les
//! pages de connexion, qui doivent revenir au site une fois finies.

/// L'adresse du site, la seule que la fenêtre affiche pour de bon.
pub const SITE: &str = "squadlog-three.vercel.app";

/// `true` : la page s'ouvre dans la fenêtre. `false` : elle part dans le
/// navigateur. `chemin` commence par `/`.
pub fn reste_dans_la_fenetre(schema: &str, hote: &str, chemin: &str) -> bool {
    match schema {
        "http" | "https" => {}
        // Pages internes du moteur (about:blank, data:…) : rien à ouvrir ailleurs.
        "about" | "data" | "blob" | "javascript" | "tauri" | "asset" | "ipc" => return true,
        // Les autres protocoles appartiennent à une application du PC :
        // steam://rungameid/… (le bouton Jouer de SQUAD//PLAY), discord://…
        // Windows les confie à la bonne application, sans passer par la fenêtre.
        _ => return false,
    }
    let hote = hote.to_ascii_lowercase();
    if hote == SITE {
        return true;
    }
    // La connexion : Supabase fait l'aller-retour, Discord et Google
    // affichent leur page de connexion puis renvoient au site.
    if hote.ends_with(".supabase.co") {
        return true;
    }
    if hote == "discord.com" {
        return ["/oauth2", "/login", "/register", "/api/"]
            .iter()
            .any(|debut| chemin.starts_with(debut));
    }
    matches!(hote.as_str(), "accounts.google.com" | "accounts.youtube.com")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_site_reste() {
        assert!(reste_dans_la_fenetre("https", "squadlog-three.vercel.app", "/bilan"));
        assert!(reste_dans_la_fenetre("https", "SQUADLOG-THREE.vercel.app", "/"));
    }

    #[test]
    fn la_connexion_reste() {
        assert!(reste_dans_la_fenetre("https", "bcemaogsgmvigaqcgduq.supabase.co", "/auth/v1/authorize"));
        assert!(reste_dans_la_fenetre("https", "discord.com", "/oauth2/authorize"));
        assert!(reste_dans_la_fenetre("https", "discord.com", "/login"));
        assert!(reste_dans_la_fenetre("https", "accounts.google.com", "/o/oauth2/v2/auth"));
    }

    #[test]
    fn le_reste_part_dans_le_navigateur() {
        assert!(!reste_dans_la_fenetre("https", "store.steampowered.com", "/app/892970/"));
        assert!(!reste_dans_la_fenetre("https", "www.youtube.com", "/watch"));
        assert!(!reste_dans_la_fenetre("https", "discord.gg", "/abcdef"));
        assert!(!reste_dans_la_fenetre("https", "discord.com", "/channels/123"));
        assert!(!reste_dans_la_fenetre("https", "squadlog-git-main-seequall.vercel.app", "/"));
        assert!(!reste_dans_la_fenetre("https", "evil-squadlog-three.vercel.app.example.com", "/"));
    }

    #[test]
    fn les_protocoles_d_applications_sortent() {
        assert!(!reste_dans_la_fenetre("steam", "rungameid", "/892970"));
        assert!(!reste_dans_la_fenetre("discord", "", "/channels/1"));
    }

    #[test]
    fn les_pages_internes_restent() {
        assert!(reste_dans_la_fenetre("about", "", "blank"));
        assert!(reste_dans_la_fenetre("data", "", "text/html,"));
    }
}
