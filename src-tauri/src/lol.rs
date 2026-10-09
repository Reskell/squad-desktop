//! Le compagnon LoL : ce que l'app lit dans le client League of Legends.
//!
//! Le client expose une API locale (la « LCU ») sur 127.0.0.1, protégée par
//! un jeton qu'il passe dans sa ligne de commande (`--app-port`,
//! `--remoting-auth-token`). Pendant une partie, le jeu expose aussi la
//! « Live Client Data API » sur 127.0.0.1:2999. Ce sont les API que Riot
//! prévoit pour les outils comme Blitz ou Porofessor : on n'y lit que ce que
//! le client montre déjà au joueur, jamais ce qu'il cache.
//!
//! L'app ne fait que transmettre au site (la fenêtre du compagnon) ; rien de
//! tout ça ne part ailleurs.

use crate::parties::Programme;
use serde::Serialize;
use serde_json::Value;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Une page de runes à importer : des numéros, rien d'autre.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PageDeRunes {
    pub nom: String,
    pub principal: u32,
    pub secondaire: u32,
    pub perks: Vec<u32>,
    /// Quelle page remplacer : "courante" (la page en cours, par défaut)
    /// ou "squad" (une page à part, nommée « SQUAD · … », qui laisse les
    /// pages de la personne intactes).
    #[serde(default)]
    pub cible: Option<String>,
}

/// Le début du nom des pages créées par le compagnon.
const PREFIXE_PAGE: &str = "SQUAD · ";

/// Un bloc d'un set d'objets à importer (« Les objets du monde »…).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BlocObjets {
    pub nom: String,
    pub objets: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccesClient {
    pub port: u16,
    pub jeton: String,
}

/// Ce que le compagnon reçoit à chaque relevé.
#[derive(Debug, Clone, Serialize, Default)]
pub struct EtatLol {
    /// Le client LoL est-il ouvert ?
    pub client: bool,
    /// La phase du client : None, Lobby, Matchmaking, ReadyCheck,
    /// ChampSelect, InProgress, PreEndOfGame, EndOfGame…
    pub phase: Option<String>,
    /// La sélection des champions en cours (session LCU), telle quelle.
    pub selection: Option<Value>,
    /// La partie en cours (Live Client Data API), telle quelle.
    pub partie: Option<Value>,
    /// Les deux équipes du chargement et de la partie (gameflow) : pour
    /// chaque joueur son compte (puuid), son champion et son poste. Les
    /// noms sont visibles de tous à partir de l'écran de chargement.
    pub rencontre: Option<Value>,
}

/// Les deux équipes d'une session gameflow, réduites à l'utile.
pub fn rencontre_de(session: &Value) -> Option<Value> {
    let partie = session.get("gameData")?;
    let equipe = |cle: &str| -> Vec<Value> {
        partie
            .get(cle)
            .and_then(Value::as_array)
            .map(|joueurs| {
                joueurs
                    .iter()
                    .map(|j| {
                        serde_json::json!({
                            "puuid": j.get("puuid").and_then(Value::as_str).unwrap_or_default(),
                            "championId": j.get("championId").and_then(Value::as_u64).unwrap_or(0),
                            "poste": j.get("selectedPosition").and_then(Value::as_str).unwrap_or_default(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let (un, deux) = (equipe("teamOne"), equipe("teamTwo"));
    if un.is_empty() && deux.is_empty() {
        return None;
    }
    Some(serde_json::json!({
        "file": partie.get("queue").and_then(|q| q.get("id")).and_then(Value::as_i64),
        "equipeUn": un,
        "equipeDeux": deux,
    }))
}

/// La valeur d'une option `--cle=valeur` dans une ligne de commande.
fn option(commande: &str, cle: &str) -> Option<String> {
    let debut = commande.find(cle)? + cle.len();
    let reste = &commande[debut..];
    let fin = reste.find([' ', '"']).unwrap_or(reste.len());
    let valeur = reste[..fin].trim_matches('"');
    (!valeur.is_empty()).then(|| valeur.to_string())
}

/// Le port et le jeton du client LoL, lus dans la ligne de commande de
/// LeagueClientUx.exe. None si le client n'est pas ouvert.
pub fn acces_client(programmes: &[Programme]) -> Option<AccesClient> {
    programmes
        .iter()
        .filter(|p| p.nom == "leagueclientux.exe")
        .find_map(|p| {
            let port = option(&p.commande_brute, "--app-port=")?.parse().ok()?;
            let jeton = option(&p.commande_brute, "--remoting-auth-token=")?;
            Some(AccesClient { port, jeton })
        })
}

/// Le client LoL vu par l'app : rafraîchi par la surveillance, et à la
/// demande (au plus toutes les 5 secondes) quand le compagnon le réclame.
pub struct Lol {
    acces: Mutex<Option<AccesClient>>,
    dernier_scan: Mutex<Option<Instant>>,
    http: Option<reqwest::Client>,
}

impl Lol {
    pub fn nouveau() -> Self {
        // Le client LoL signe ses réponses avec un certificat à lui : on
        // l'accepte, et seulement pour 127.0.0.1 (voir `lire`).
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder()
            .tls_danger_accept_invalid_certs(true)
            .timeout(Duration::from_secs(3))
            .no_proxy()
            .build()
            .ok();
        Lol {
            acces: Mutex::new(None),
            dernier_scan: Mutex::new(None),
            http,
        }
    }

    pub fn noter(&self, acces: Option<AccesClient>) {
        *self.acces.lock().unwrap() = acces;
        *self.dernier_scan.lock().unwrap() = Some(Instant::now());
    }

    pub fn acces(&self) -> Option<AccesClient> {
        self.acces.lock().unwrap().clone()
    }

    /// Faut-il relire les programmes (le client vient peut-être d'ouvrir) ?
    pub fn scan_du(&self) -> bool {
        self.acces().is_none()
            && self
                .dernier_scan
                .lock()
                .unwrap()
                .map(|t| t.elapsed() > Duration::from_secs(5))
                .unwrap_or(true)
    }

    async fn lire(&self, url: String, acces: Option<&AccesClient>) -> Option<Value> {
        let http = self.http.as_ref()?;
        // Garde-fou : ce client n'accepte n'importe quel certificat que
        // parce qu'il ne parle qu'à cette machine.
        if !url.starts_with("https://127.0.0.1:") {
            return None;
        }
        let mut requete = http.get(url);
        if let Some(a) = acces {
            requete = requete.basic_auth("riot", Some(&a.jeton));
        }
        let reponse = requete.send().await.ok()?;
        if !reponse.status().is_success() {
            return None;
        }
        reponse.json::<Value>().await.ok()
    }

    /// Écrire dans le client LoL (POST, PUT, PATCH, DELETE), seulement sur
    /// les quelques chemins que le compagnon utilise (voir plus bas).
    async fn envoyer(&self, methode: reqwest::Method, chemin: &str, corps: Option<Value>) -> Result<Option<Value>, String> {
        let http = self.http.as_ref().ok_or("Client HTTP indisponible")?;
        let acces = self.acces().ok_or("Le client LoL n'est pas ouvert.")?;
        let mut requete = http
            .request(methode, format!("https://127.0.0.1:{}{chemin}", acces.port))
            .basic_auth("riot", Some(&acces.jeton));
        if let Some(corps) = corps {
            requete = requete.json(&corps);
        }
        let reponse = requete.send().await.map_err(|_| "Le client LoL ne répond pas.".to_string())?;
        if !reponse.status().is_success() {
            return Err(format!("Le client LoL a refusé ({}).", reponse.status().as_u16()));
        }
        Ok(reponse.json::<Value>().await.ok())
    }

    /// Remplace la page de runes en cours par celle du compagnon. Seuls des
    /// numéros de runes passent : la page est reconstruite ici.
    pub async fn importer_runes(&self, page: &PageDeRunes) -> Result<(), String> {
        if page.perks.len() != 9 {
            return Err("Page de runes incomplète.".into());
        }
        if page.cible.as_deref() == Some("squad") {
            // Une page à part : on remplace l'ancienne page SQUAD, jamais
            // celles de la personne.
            if let Ok(Some(Value::Array(pages))) = self.envoyer(reqwest::Method::GET, "/lol-perks/v1/pages", None).await {
                for p in pages {
                    let a_nous = p.get("name").and_then(Value::as_str).is_some_and(|n| n.starts_with(PREFIXE_PAGE));
                    let modifiable = p.get("isEditable").and_then(Value::as_bool).unwrap_or(false);
                    if let (true, true, Some(id)) = (a_nous, modifiable, p.get("id").and_then(Value::as_u64)) {
                        let _ = self
                            .envoyer(reqwest::Method::DELETE, &format!("/lol-perks/v1/pages/{id}"), None)
                            .await;
                    }
                }
            }
        } else if let Ok(Some(actuelle)) = self.envoyer(reqwest::Method::GET, "/lol-perks/v1/currentpage", None).await {
            // La page en cours, si on peut la modifier, laisse sa place (le
            // client limite le nombre de pages).
            let modifiable = actuelle.get("isEditable").and_then(Value::as_bool).unwrap_or(false);
            if let (true, Some(id)) = (modifiable, actuelle.get("id").and_then(Value::as_u64)) {
                let _ = self
                    .envoyer(reqwest::Method::DELETE, &format!("/lol-perks/v1/pages/{id}"), None)
                    .await;
            }
        }
        let corps = serde_json::json!({
            "name": format!("{PREFIXE_PAGE}{}", page.nom.chars().take(20).collect::<String>()),
            "primaryStyleId": page.principal,
            "subStyleId": page.secondaire,
            "selectedPerkIds": page.perks,
            "current": true
        });
        self.envoyer(reqwest::Method::POST, "/lol-perks/v1/pages", Some(corps))
            .await
            .map(|_| ())
            .map_err(|e| {
                if page.cible.as_deref() == Some("squad") {
                    format!("{e} (plus de place pour une page ? Libère-en une, ou choisis « page en cours » dans les réglages.)")
                } else {
                    e
                }
            })
    }

    /// Choisit les deux sorts d'invocateur pendant la sélection.
    pub async fn importer_sorts(&self, premier: u32, second: u32) -> Result<(), String> {
        let corps = serde_json::json!({ "spell1Id": premier, "spell2Id": second });
        self.envoyer(
            reqwest::Method::PATCH,
            "/lol-champ-select/v1/session/my-selection",
            Some(corps),
        )
        .await
        .map(|_| ())
    }

    /// Range un set d'objets « SQUAD · … » dans le client (le set d'avant du
    /// compagnon est remplacé ; ceux de la personne ne bougent pas).
    pub async fn importer_objets(&self, titre: &str, champion: u32, blocs: &[BlocObjets]) -> Result<(), String> {
        let moi = self
            .envoyer(reqwest::Method::GET, "/lol-summoner/v1/current-summoner", None)
            .await?
            .ok_or("Invocateur inconnu.")?;
        let id = moi
            .get("summonerId")
            .and_then(Value::as_u64)
            .ok_or("Invocateur inconnu.")?;
        let chemin = format!("/lol-item-sets/v1/item-sets/{id}/sets");
        let mut tout = match self.envoyer(reqwest::Method::GET, &chemin, None).await? {
            Some(v @ Value::Object(_)) => v,
            _ => serde_json::json!({}),
        };
        let mut sets: Vec<Value> = tout
            .get("itemSets")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|s| {
                !s.get("title")
                    .and_then(Value::as_str)
                    .is_some_and(|t| t.starts_with(PREFIXE_PAGE))
            })
            .collect();
        sets.push(set_d_objets(titre, champion, blocs));
        tout["itemSets"] = Value::Array(sets);
        if tout.get("accountId").is_none() {
            tout["accountId"] = serde_json::json!(id);
        }
        tout["timestamp"] = serde_json::json!(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0));
        self.envoyer(reqwest::Method::PUT, &chemin, Some(tout)).await.map(|_| ())
    }

    /// Un relevé complet pour le compagnon.
    pub async fn etat(&self) -> EtatLol {
        let Some(acces) = self.acces() else {
            return EtatLol::default();
        };
        let base = format!("https://127.0.0.1:{}", acces.port);
        let Some(phase) = self
            .lire(format!("{base}/lol-gameflow/v1/gameflow-phase"), Some(&acces))
            .await
        else {
            // Le client ne répond plus : il a été fermé (ou redémarre).
            self.noter(None);
            return EtatLol::default();
        };
        let phase = phase.as_str().map(str::to_string);
        let mut etat = EtatLol {
            client: true,
            phase: phase.clone(),
            ..Default::default()
        };
        match phase.as_deref() {
            Some("ChampSelect") => {
                etat.selection = self
                    .lire(format!("{base}/lol-champ-select/v1/session"), Some(&acces))
                    .await;
            }
            Some("GameStart") | Some("InProgress") | Some("Reconnect") => {
                if let Some(session) = self
                    .lire(format!("{base}/lol-gameflow/v1/session"), Some(&acces))
                    .await
                {
                    etat.rencontre = rencontre_de(&session);
                }
                if phase.as_deref() != Some("GameStart") {
                    etat.partie = self
                        .lire("https://127.0.0.1:2999/liveclientdata/allgamedata".to_string(), None)
                        .await;
                }
            }
            _ => {}
        }
        etat
    }
}

/// Un set d'objets au format du client LoL (blocs d'objets, un exemplaire chacun).
pub fn set_d_objets(titre: &str, champion: u32, blocs: &[BlocObjets]) -> Value {
    let blocs: Vec<Value> = blocs
        .iter()
        .filter(|b| !b.objets.is_empty())
        .map(|b| {
            serde_json::json!({
                "type": b.nom.chars().take(40).collect::<String>(),
                "hideIfSummonerSpell": "",
                "showIfSummonerSpell": "",
                "items": b.objets.iter().take(12).map(|o| serde_json::json!({ "id": o.to_string(), "count": 1 })).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::json!({
        "title": format!("{PREFIXE_PAGE}{}", titre.chars().take(30).collect::<String>()),
        "associatedChampions": [champion],
        "associatedMaps": [11],
        "blocks": blocs,
        "map": "SR",
        "mode": "CLASSIC",
        "preferredItemSlots": [],
        "sortrank": 0,
        "startedFrom": "blank",
        "type": "custom",
        "uid": format!("squad-compagnon-{champion}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rencontre_garde_l_utile() {
        let session = serde_json::json!({
            "gameData": {
                "queue": { "id": 420 },
                "teamOne": [{ "puuid": "a", "championId": 103, "selectedPosition": "MIDDLE", "summonerName": "x" }],
                "teamTwo": [{ "puuid": "b", "championId": 238 }]
            }
        });
        let r = rencontre_de(&session).unwrap();
        assert_eq!(r["file"], 420);
        assert_eq!(r["equipeUn"][0]["puuid"], "a");
        assert_eq!(r["equipeUn"][0]["poste"], "MIDDLE");
        assert_eq!(r["equipeDeux"][0]["championId"], 238);
        assert!(r["equipeUn"][0].get("summonerName").is_none());
        assert!(rencontre_de(&serde_json::json!({})).is_none());
    }

    #[test]
    fn set_d_objets_au_format_du_client() {
        let set = set_d_objets(
            "Ahri",
            103,
            &[
                BlocObjets { nom: "Le monde".into(), objets: vec![3089, 3157] },
                BlocObjets { nom: "Vide".into(), objets: vec![] },
            ],
        );
        assert_eq!(set["title"], "SQUAD · Ahri");
        assert_eq!(set["associatedChampions"][0], 103);
        assert_eq!(set["blocks"].as_array().unwrap().len(), 1);
        assert_eq!(set["blocks"][0]["items"][1]["id"], "3157");
    }

    #[test]
    fn lit_le_port_et_le_jeton() {
        let p = Programme {
            nom: "leagueclientux.exe".into(),
            commande_brute: r#""C:/Riot Games/League of Legends/LeagueClientUx.exe" "--riotclient-auth-token=abc" "--app-port=51234" "--remoting-auth-token=AbC-12_x" --locale=fr_FR"#.into(),
            ..Default::default()
        };
        assert_eq!(
            acces_client(&[p]),
            Some(AccesClient { port: 51234, jeton: "AbC-12_x".into() })
        );
    }

    #[test]
    fn rien_sans_le_client() {
        let p = Programme { nom: "explorer.exe".into(), ..Default::default() };
        assert_eq!(acces_client(&[p]), None);
    }
}
