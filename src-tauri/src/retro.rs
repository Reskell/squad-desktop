//! SQUAD//RETRO dans l'app : les jeux rétro de la personne, lancés en un clic.
//!
//! Le moteur (dépôt Squad-retro) fait le travail : reconnaître les jeux,
//! écrire les configs, préparer les commandes, installer les émulateurs,
//! tenir la room. Ici, l'app garde l'état de la personne, télécharge, lance
//! les processus et prévient la fenêtre (événements « retro-… »).
//!
//! Sécurité : la fenêtre ne peut lancer que des fichiers trouvés par le
//! dernier scan, et l'app ne télécharge que les adresses des manifestes
//! publiés dans le dépôt Squad-retro, vérifiées par empreinte.
//!
//! Tout vit dans `<données de l'app>/retro/` et `retro.json`.

use serde::{Deserialize, Serialize};
use squad_retro::avatar::{self, Avatar, Compte, Comptes, Croise};
use squad_retro::bibliotheque::{self, Bibliotheque, Options};
use squad_retro::jaquettes;
use squad_retro::catalogue::{Catalogue, Statut};
use squad_retro::historique::{Historique, Partie};
use squad_retro::installation::{self, Etat};
use squad_retro::manettes::{self, BoutonPad, Etape};
use squad_retro::mods::{self, CatalogueMods, Installes};
use squad_retro::netplay::{self, Invitation, Place, Role, Room, Session};
use squad_retro::region::ORDRE_PAR_DEFAUT;
use squad_retro::room_reseau::{self, ClientRoom, Nouvelle, ServeurRoom};
use squad_retro::versions::{Action, Canal, Validees};
use squad_retro::{diagnostic, lancement, sauvegarde, Console, Emplacements, Fiche, Profil, Region};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// Ce que la personne a choisi.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReglagesRetro {
    /// Le dossier qui contient `nes/`, `snes/`… (et `dats/` pour les bases).
    pub dossier_jeux: Option<String>,
    /// Un RetroArch déjà installé (sinon celui installé par SQUAD).
    pub retroarch: Option<String>,
    /// Le pseudo affiché en netplay.
    #[serde(default)]
    pub pseudo: Option<String>,
    #[serde(default)]
    pub canal: Canal,
    /// La révision des versions validées déjà appliquée.
    #[serde(default)]
    pub revision: u32,
    /// La palette de l'interface (« squad », « gamecube »…).
    #[serde(default)]
    pub theme: Option<String>,
    /// Vrai pour le thème clair.
    #[serde(default)]
    pub clair: bool,
    /// Vrai pour couper les sons de menu.
    #[serde(default)]
    pub muet: bool,
    /// Vrai pour entrer directement avec le dernier profil, sans l'écran de choix.
    #[serde(default)]
    pub entrer_direct: bool,
}

#[derive(Serialize, Clone)]
pub struct VarianteVisible {
    pub chemin: String,
    pub nom: String,
    pub region: Region,
    pub reconnu: bool,
    /// Vrai si c'est une variante installée par un mod.
    pub mod_installe: bool,
}

#[derive(Serialize, Clone)]
pub struct FicheVisible {
    pub console: Console,
    pub console_nom: &'static str,
    pub titre: String,
    pub statut: Statut,
    pub statut_texte: &'static str,
    pub notes: String,
    pub joueurs: u8,
    pub netplay: bool,
    /// Temps joué, en secondes.
    pub temps: u64,
    /// L'adresse de la jaquette (base publique, rien n'est livré avec l'app).
    pub jaquette: String,
    pub variantes: Vec<VarianteVisible>,
}

#[derive(Serialize, Clone)]
pub struct ConsoleVisible {
    pub console: Console,
    pub nom: &'static str,
    /// Ce qui manque, en clair (vide = prête).
    pub manques: Vec<String>,
    /// Vrai si l'installation automatique peut tout régler.
    pub installable: bool,
    pub configuree: bool,
}

#[derive(Serialize, Clone)]
pub struct EtatVisible {
    pub reglages: ReglagesRetro,
    pub consoles: Vec<ConsoleVisible>,
    pub versions: BTreeMap<String, String>,
    pub dossier_bios: String,
    pub manettes: Vec<String>,
    pub comptes: Comptes,
}

#[derive(Serialize, Clone)]
pub struct ProgresInstallation {
    pub composant: String,
    pub etape: String,
    pub fait: usize,
    pub total: usize,
}

pub struct Retro {
    reglages: Mutex<ReglagesRetro>,
    fichier: Option<PathBuf>,
    donnees: PathBuf,
    derniere: Mutex<Bibliotheque>,
    profils: Mutex<BTreeMap<Console, Profil>>,
    historique: Mutex<Historique>,
    catalogue: Mutex<Catalogue>,
    catalogue_mods: Mutex<CatalogueMods>,
    mods_installes: Mutex<Installes>,
    erreurs: Mutex<Vec<String>>,
    partie_test: Mutex<Option<Child>>,
    manettes: Mutex<Vec<String>>,
    serveur: Mutex<Option<Arc<ServeurRoom>>>,
    client: Mutex<Option<Arc<ClientRoom>>>,
    /// La partie prévue par la room : lancée à la fin du compte à rebours.
    prevue: Mutex<Option<(String, Session)>>,
    /// Vrai pendant une installation : une seule à la fois.
    installation: std::sync::atomic::AtomicBool,
    comptes: Mutex<Comptes>,
    croises: Mutex<Vec<Croise>>,
}

fn lire_json<T: for<'a> Deserialize<'a> + Default>(f: &Path) -> T {
    fs::read_to_string(f).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn ecrire_json<T: Serialize>(f: &Path, v: &T) {
    if let Some(d) = f.parent() {
        let _ = fs::create_dir_all(d);
    }
    if let Ok(t) = serde_json::to_string_pretty(v) {
        let _ = fs::write(f, t);
    }
}

impl Retro {
    pub fn charger(donnees_app: Option<PathBuf>) -> Self {
        let fichier = donnees_app.as_ref().map(|d| d.join("retro.json"));
        let reglages = fichier.as_deref().map(lire_json).unwrap_or_default();
        let donnees = donnees_app.unwrap_or_else(|| PathBuf::from(".")).join("retro");
        let profils: BTreeMap<Console, Profil> = lire_json(&donnees.join("profils.json"));
        let comptes: Comptes = lire_json(&donnees.join("comptes.json"));
        let croises: Vec<Croise> = lire_json(&donnees.join("croises.json"));
        let catalogue = fs::read_to_string(donnees.join("catalogue.json"))
            .ok()
            .and_then(|t| Catalogue::lire(&t).ok())
            .unwrap_or_default();
        Retro {
            reglages: Mutex::new(reglages),
            fichier,
            historique: Mutex::new(Historique::charger(&donnees.join("historique.json"))),
            catalogue_mods: Mutex::new(lire_json(&donnees.join("mods-catalogue.json"))),
            mods_installes: Mutex::new(Installes::charger(&donnees.join("variantes").join("mods.json"))),
            catalogue: Mutex::new(catalogue),
            profils: Mutex::new(profils),
            donnees,
            derniere: Mutex::new(Bibliotheque::default()),
            erreurs: Mutex::new(Vec::new()),
            partie_test: Mutex::new(None),
            manettes: Mutex::new(Vec::new()),
            serveur: Mutex::new(None),
            client: Mutex::new(None),
            prevue: Mutex::new(None),
            installation: Default::default(),
            comptes: Mutex::new(comptes),
            croises: Mutex::new(croises),
        }
    }

    pub fn reglages(&self) -> ReglagesRetro {
        self.reglages.lock().unwrap().clone()
    }

    fn modifier(&self, f: impl FnOnce(&mut ReglagesRetro)) {
        f(&mut self.reglages.lock().unwrap());
        if let Some(fichier) = &self.fichier {
            ecrire_json(fichier, &self.reglages());
        }
    }

    pub fn choisir_dossier_jeux(&self, dossier: &Path) {
        self.modifier(|r| r.dossier_jeux = Some(dossier.to_string_lossy().into_owned()));
    }
    pub fn choisir_retroarch(&self, exe: Option<&Path>) {
        self.modifier(|r| r.retroarch = exe.map(|e| e.to_string_lossy().into_owned()));
    }
    pub fn choisir_pseudo(&self, pseudo: &str) {
        let p: String = pseudo.trim().chars().filter(|c| !c.is_control()).take(24).collect();
        self.modifier(|r| r.pseudo = (!p.is_empty()).then_some(p));
    }
    pub fn choisir_canal(&self, canal: Canal) {
        self.modifier(|r| {
            r.canal = canal;
            r.revision = 0;
        });
    }

    pub fn emplacements(&self) -> Emplacements {
        match self.reglages().retroarch {
            Some(exe) => Emplacements::avec_retroarch(Path::new(&exe), &self.donnees),
            None => Emplacements::geres(&self.donnees),
        }
    }

    pub fn noter_erreur(&self, e: &str) {
        let mut erreurs = self.erreurs.lock().unwrap();
        erreurs.push(e.to_string());
        let trop = erreurs.len().saturating_sub(50);
        erreurs.drain(..trop);
    }

    fn pseudo(&self) -> String {
        self.reglages().pseudo.unwrap_or_else(|| "Joueur".into())
    }

    pub fn profil(&self, console: Console) -> Profil {
        self.profils.lock().unwrap().get(&console).cloned().unwrap_or_else(|| Profil::par_defaut(console))
    }

    pub fn enregistrer_profil(&self, profil: Profil) -> Result<(), String> {
        if let Some(m) = &profil.mapping {
            let conflits = m.conflits();
            if !conflits.is_empty() {
                return Err(format!("Un même bouton sert deux fois ({conflits:?}) : corrige le mapping."));
            }
        }
        let mut profils = self.profils.lock().unwrap();
        profils.insert(profil.console, profil);
        ecrire_json(&self.donnees.join("profils.json"), &*profils);
        Ok(())
    }

    pub fn etat(&self) -> EtatVisible {
        let ou = self.emplacements();
        let profils = self.profils.lock().unwrap().clone();
        EtatVisible {
            reglages: self.reglages(),
            consoles: Console::TOUTES
                .into_iter()
                .map(|c| {
                    let m = lancement::manques(c, &ou);
                    ConsoleVisible {
                        console: c,
                        nom: c.nom(),
                        installable: m.iter().all(|x| x.installable()),
                        manques: m.iter().map(|x| x.message(c)).collect(),
                        configuree: profils.get(&c).map(|p| p.configure).unwrap_or(false),
                    }
                })
                .collect(),
            versions: Etat::charger(&ou).versions(),
            dossier_bios: ou.bios().to_string_lossy().into_owned(),
            manettes: self.manettes.lock().unwrap().clone(),
            comptes: self.comptes.lock().unwrap().clone(),
        }
    }

    // ----- Profils SQUAD, avatars, plaza, thème -----

    fn ecrire_comptes(&self) {
        ecrire_json(&self.donnees.join("comptes.json"), &*self.comptes.lock().unwrap());
    }

    /// Crée ou modifie un profil ; le nouveau profil devient le profil actif.
    pub fn enregistrer_compte(&self, compte: Compte) -> Result<Compte, String> {
        let nouveau = compte.id.is_empty();
        let c = self.comptes.lock().unwrap().enregistrer(compte, || {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
            format!("p{t:x}")
        })?;
        self.ecrire_comptes();
        let actif = self.comptes.lock().unwrap().actif.clone();
        if nouveau || actif.as_deref() == Some(c.id.as_str()) {
            self.choisir_compte(Some(&c.id))?;
        }
        Ok(c)
    }

    pub fn supprimer_compte(&self, id: &str) {
        self.comptes.lock().unwrap().supprimer(id);
        self.ecrire_comptes();
    }

    /// Entre avec un profil : son pseudo devient le pseudo en ligne.
    pub fn choisir_compte(&self, id: Option<&str>) -> Result<(), String> {
        let pseudo = {
            let mut c = self.comptes.lock().unwrap();
            match id {
                Some(id) => {
                    let p = c.liste.iter().find(|x| x.id == id).map(|x| x.pseudo.clone()).ok_or("Profil introuvable.")?;
                    c.actif = Some(id.to_string());
                    Some(p)
                }
                None => {
                    c.actif = None;
                    None
                }
            }
        };
        self.ecrire_comptes();
        if let Some(p) = pseudo {
            self.modifier(|r| r.pseudo = Some(p));
        }
        Ok(())
    }

    fn avatar(&self) -> Avatar {
        self.comptes.lock().unwrap().actif().map(|c| c.avatar).unwrap_or_else(|| Avatar::depuis(&self.pseudo()))
    }

    pub fn choisir_theme(&self, theme: Option<String>, clair: bool, muet: bool, entrer_direct: bool) {
        let theme = theme.filter(|t| t.len() <= 24 && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        self.modifier(|r| {
            r.theme = theme;
            r.clair = clair;
            r.muet = muet;
            r.entrer_direct = entrer_direct;
        });
    }

    /// Les joueurs croisés en room, le plus récent d'abord.
    pub fn croises(&self) -> Vec<Croise> {
        self.croises.lock().unwrap().clone()
    }

    fn noter_croises(&self, room: &Room) {
        let moi = self.pseudo();
        let vu = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
        let mut liste = self.croises.lock().unwrap();
        let avant = liste.clone();
        for p in room.places.iter().filter(|p| !p.pseudo.eq_ignore_ascii_case(&moi)) {
            avatar::noter_croise(
                &mut liste,
                Croise { pseudo: p.pseudo.clone(), avatar: p.avatar.unwrap_or_else(|| Avatar::depuis(&p.pseudo)), jeu: room.titre.clone(), vu },
            );
        }
        if *liste != avant {
            ecrire_json(&self.donnees.join("croises.json"), &*liste);
        }
    }

    /// Scanne le dossier des jeux et les variantes installées par des mods.
    pub fn scanner(&self) -> Result<Vec<FicheVisible>, String> {
        let dossier = self.reglages().dossier_jeux.ok_or_else(|| "Choisis d'abord le dossier de tes jeux.".to_string())?;
        let racine = PathBuf::from(dossier);
        if !racine.is_dir() {
            return Err(format!("Le dossier {} est introuvable.", racine.display()));
        }
        let bases = bibliotheque::charger_bases(&racine.join("dats"));
        let options = Options { bases: &bases, ordre: &ORDRE_PAR_DEFAUT };
        let mut biblio = Bibliotheque::scanner(&racine, &options);
        let variantes = Bibliotheque::scanner(&self.donnees.join("variantes"), &options);
        for f in variantes.fiches {
            match biblio.fiches.iter_mut().find(|x| x.console == f.console && x.titre.eq_ignore_ascii_case(&f.titre)) {
                Some(x) => x.variantes.extend(f.variantes),
                None => biblio.fiches.push(f),
            }
        }
        biblio.fiches.sort_by_key(|a| (a.console, a.titre.to_lowercase()));
        *self.derniere.lock().unwrap() = biblio;
        Ok(self.visibles(|_| true))
    }

    fn visibles(&self, filtre: impl Fn(&Fiche) -> bool) -> Vec<FicheVisible> {
        let biblio = self.derniere.lock().unwrap();
        biblio.fiches.iter().filter(|f| filtre(f)).map(|f| self.visible(f)).collect()
    }

    fn visible(&self, fiche: &Fiche) -> FicheVisible {
        let catalogue = self.catalogue.lock().unwrap();
        let info = catalogue.info(fiche.console, &fiche.titre);
        let variantes_mods = self.donnees.join("variantes");
        FicheVisible {
            console: fiche.console,
            console_nom: fiche.console.nom(),
            titre: fiche.titre.clone(),
            statut: info.map(|i| i.statut).unwrap_or_default(),
            statut_texte: info.map(|i| i.statut).unwrap_or_default().texte(),
            notes: info.map(|i| i.notes.clone()).unwrap_or_default(),
            joueurs: info.map(|i| i.joueurs).unwrap_or(1),
            netplay: info.map(|i| i.netplay).unwrap_or(false),
            temps: self.historique.lock().unwrap().temps(fiche.console, &fiche.titre),
            jaquette: {
                let v = fiche.preferee();
                let nom = if v.reconnu {
                    v.nom.clone()
                } else {
                    v.chemin.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| v.nom.clone())
                };
                jaquettes::jaquette(fiche.console, &v.chemin, &nom)
            },
            variantes: fiche
                .variantes
                .iter()
                .map(|v| VarianteVisible {
                    chemin: v.chemin.to_string_lossy().into_owned(),
                    nom: v.nom.clone(),
                    region: v.region,
                    reconnu: v.reconnu,
                    mod_installe: v.chemin.starts_with(&variantes_mods),
                })
                .collect(),
        }
    }

    pub fn rechercher(&self, texte: &str) -> Vec<FicheVisible> {
        let biblio = self.derniere.lock().unwrap().clone();
        biblio.rechercher(texte).into_iter().map(|f| self.visible(f)).collect()
    }

    /// Un jeu au hasard, avec au moins `joueurs` joueurs (1 = tous les jeux).
    pub fn au_hasard(&self, joueurs: u8, console: Option<Console>) -> Option<FicheVisible> {
        let biblio = self.derniere.lock().unwrap().clone();
        let catalogue = self.catalogue.lock().unwrap().clone();
        let graine = room_reseau::maintenant_ms();
        biblio
            .au_hasard(
                |f| {
                    console.map(|c| c == f.console).unwrap_or(true)
                        && catalogue.info(f.console, &f.titre).map(|i| i.joueurs).unwrap_or(1) >= joueurs
                },
                graine,
            )
            .map(|f| self.visible(f))
    }

    pub fn reprendre(&self) -> Vec<Partie> {
        self.historique.lock().unwrap().reprendre(5).into_iter().cloned().collect()
    }

    pub fn doublons(&self) -> Vec<Vec<String>> {
        self.derniere
            .lock()
            .unwrap()
            .doublons()
            .into_iter()
            .map(|g| g.into_iter().map(|p| p.to_string_lossy().into_owned()).collect())
            .collect()
    }

    /// Retrouve un fichier du dernier scan (le seul moyen de lancer quelque chose).
    fn trouver(&self, chemin: &str) -> Result<(Console, String, squad_retro::Variante), String> {
        let biblio = self.derniere.lock().unwrap();
        biblio
            .fiches
            .iter()
            .flat_map(|f| f.variantes.iter().map(move |v| (f, v)))
            .find(|(_, v)| v.chemin.to_string_lossy() == chemin)
            .map(|(f, v)| (f.console, f.titre.clone(), v.clone()))
            .ok_or_else(|| "Ce jeu n'est plus dans la bibliothèque : relance le scan.".to_string())
    }

    /// Lance un jeu (en solo ou en netplay). Retourne le processus lancé.
    pub fn lancer(self: &Arc<Self>, chemin: &str, session: Option<Session>, app: Option<AppHandle>) -> Result<(), String> {
        let resultat = self.lancer_sans_noter(chemin, session, app);
        if let Err(e) = &resultat {
            self.noter_erreur(e);
        }
        resultat
    }

    fn lancer_sans_noter(self: &Arc<Self>, chemin: &str, session: Option<Session>, app: Option<AppHandle>) -> Result<(), String> {
        let (console, titre, variante) = self.trouver(chemin)?;
        let ou = self.emplacements();
        lancement::verifier(console, &ou)?;
        let options = self
            .catalogue
            .lock()
            .unwrap()
            .info(console, &titre)
            .map(|i| i.options.clone())
            .unwrap_or_default();
        let profil = self.profil(console).avec_options(&options);
        lancement::preparer(console, &ou, &profil, session.as_ref())
            .map_err(|e| format!("Impossible d'écrire la config de {} : {e}", console.nom()))?;
        let commande = lancement::commande(console, &variante.chemin, &ou, session.as_ref());
        let mut enfant = commande
            .vers_process()
            .spawn()
            .map_err(|e| format!("Impossible de lancer {} : {e}", commande.programme.display()))?;

        let debut = room_reseau::maintenant_ms();
        self.historique.lock().unwrap().commencer(Partie {
            console,
            titre: titre.clone(),
            chemin: variante.chemin.clone(),
            debut_ms: debut,
            duree_s: 0,
        });
        self.ecrire_historique();
        if let Some(app) = &app {
            let _ = app.emit("retro-partie", serde_json::json!({ "titre": titre, "en_cours": true }));
        }
        // Un fil attend la fin de la partie pour noter le temps joué.
        let moi = self.clone();
        let chemin_partie = variante.chemin.clone();
        std::thread::spawn(move || {
            let _ = enfant.wait();
            moi.historique.lock().unwrap().terminer(&chemin_partie, room_reseau::maintenant_ms());
            moi.ecrire_historique();
            if let Some(app) = app {
                let _ = app.emit("retro-partie", serde_json::json!({ "titre": titre, "en_cours": false }));
            }
        });
        Ok(())
    }

    fn ecrire_historique(&self) {
        let _ = self.historique.lock().unwrap().ecrire(&self.donnees.join("historique.json"));
    }

    /// La partie test de l'assistant : le premier jeu de la console, 30 secondes.
    pub fn partie_test(self: &Arc<Self>, console: Console) -> Result<String, String> {
        let (chemin, titre) = {
            let biblio = self.derniere.lock().unwrap();
            let f = biblio.de(console).next().ok_or_else(|| format!("Aucun jeu {} pour la partie test.", console.nom()))?;
            (f.preferee().chemin.clone(), f.titre.clone())
        };
        let ou = self.emplacements();
        lancement::verifier(console, &ou)?;
        lancement::preparer(console, &ou, &self.profil(console), None).map_err(|e| e.to_string())?;
        let enfant = lancement::commande(console, &chemin, &ou, None)
            .vers_process()
            .spawn()
            .map_err(|e| e.to_string())?;
        *self.partie_test.lock().unwrap() = Some(enfant);
        let moi = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(30));
            if let Some(mut e) = moi.partie_test.lock().unwrap().take() {
                let _ = e.kill();
            }
        });
        Ok(titre)
    }

    pub fn arreter_partie_test(&self) {
        if let Some(mut e) = self.partie_test.lock().unwrap().take() {
            let _ = e.kill();
        }
    }

    // ----- Installation et mises à jour -----

    /// Récupère les versions validées et le catalogue, puis installe ce qui manque.
    pub async fn installer_tout(self: Arc<Self>, app: AppHandle) -> Result<Vec<String>, String> {
        use std::sync::atomic::Ordering;
        if self.installation.swap(true, Ordering::SeqCst) {
            return Err("Une installation est déjà en cours : attends qu'elle se termine.".into());
        }
        let r = self.clone().installer_tout_sans_verrou(app).await;
        self.installation.store(false, Ordering::SeqCst);
        r
    }

    async fn installer_tout_sans_verrou(self: Arc<Self>, app: AppHandle) -> Result<Vec<String>, String> {
        let canal = self.reglages().canal;
        let texte = match telecharger_texte(&canal.url_versions()).await {
            Err(e) if canal == Canal::Stable && e.contains("404") => {
                return Err("Aucune version n'est encore validée pour tout le monde. \
                            Passe sur le canal Bêta dans Réglages pour installer la version de test."
                    .to_string())
            }
            r => r?,
        };
        let validees = Validees::lire(&texte)?;
        if let Ok(t) = telecharger_texte(&Canal::url_catalogue()).await {
            if let Ok(c) = Catalogue::lire(&t) {
                let _ = fs::write(self.donnees.join("catalogue.json"), &t);
                *self.catalogue.lock().unwrap() = c;
            }
        }
        if let Ok(t) = telecharger_texte(&format!("{}/mods.json", squad_retro::versions::BASE_MANIFESTES)).await {
            if let Ok(c) = CatalogueMods::lire(&t) {
                ecrire_json(&self.donnees.join("mods-catalogue.json"), &c);
                *self.catalogue_mods.lock().unwrap() = c;
            }
        }
        let ou = self.emplacements();
        let mut actions = validees.a_faire(&Etat::charger(&ou).versions());
        // Un composant à la bonne version mais abîmé est réinstallé.
        for nom in installation::a_reparer(&ou) {
            let deja = actions.iter().any(|a| matches!(a, Action::Installer { nom: n, .. } | Action::MettreAJour { nom: n, .. } if *n == nom));
            if let (false, Some(composant)) = (deja, validees.composants.get(&nom)) {
                actions.push(Action::Installer { nom, composant: composant.clone() });
            }
        }
        // Les cores vont dans le dossier de RetroArch : RetroArch d'abord.
        actions.sort_by_key(|a| match a {
            Action::Installer { nom, .. } | Action::MettreAJour { nom, .. } => (nom != "retroarch", nom.clone()),
        });
        // Avec un RetroArch personnel, on n'installe pas celui de SQUAD.
        if self.reglages().retroarch.is_some() {
            actions.retain(|a| !matches!(a, Action::Installer { nom, .. } | Action::MettreAJour { nom, .. } if nom == "retroarch"));
        }
        let total = actions.len();
        let mut faits = Vec::new();
        for (i, action) in actions.into_iter().enumerate() {
            let (nom, composant) = match action {
                Action::Installer { nom, composant } | Action::MettreAJour { nom, composant, .. } => (nom, composant),
            };
            let progres = |etape: &str| {
                let _ = app.emit(
                    "retro-installation",
                    ProgresInstallation { composant: nom.clone(), etape: etape.into(), fait: i, total },
                );
            };
            progres("Téléchargement");
            let octets = telecharger(&composant.url).await?;
            progres("Installation");
            let ou2 = ou.clone();
            let nom2 = nom.clone();
            tauri::async_runtime::spawn_blocking(move || installation::installer(&nom2, &composant, &octets, &ou2))
                .await
                .map_err(|_| "L'installation a planté.".to_string())??;
            faits.push(nom);
        }
        self.modifier(|r| r.revision = validees.revision);
        let _ = app.emit(
            "retro-installation",
            ProgresInstallation { composant: String::new(), etape: "Terminé".into(), fait: total, total },
        );
        Ok(faits)
    }

    pub fn retour_arriere(&self, nom: &str) -> Result<String, String> {
        installation::retour_arriere(nom, &self.emplacements())
    }

    pub fn integrite(&self) -> BTreeMap<String, Vec<String>> {
        let ou = self.emplacements();
        Etat::charger(&ou)
            .composants
            .keys()
            .map(|n| (n.clone(), installation::verifier_integrite(n, &ou)))
            .collect()
    }

    // ----- Mods -----

    /// Les mods disponibles pour une version d'un jeu.
    pub fn mods_pour(&self, chemin: &str) -> Result<Vec<mods::ModRetro>, String> {
        let (_, _, v) = self.trouver(chemin)?;
        let sha1 = v.empreintes.map(|e| e.sha1).unwrap_or_default();
        Ok(self.catalogue_mods.lock().unwrap().pour(&sha1).into_iter().cloned().collect())
    }

    /// Installe (ou met à jour) un mod sur une version d'un jeu.
    pub async fn installer_mod(self: Arc<Self>, id: String, chemin: String) -> Result<String, String> {
        let (console, _, v) = self.trouver(&chemin)?;
        let m = self
            .catalogue_mods
            .lock()
            .unwrap()
            .mods
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .ok_or("Mod inconnu.")?;
        let patch = telecharger(&m.url).await?;
        let dossier = self.donnees.join("variantes").join(console.dossier());
        let moi = self.clone();
        let fichier = tauri::async_runtime::spawn_blocking(move || {
            let mut installes = moi.mods_installes.lock().unwrap();
            let r = mods::installer(&m, &v.chemin, &patch, &dossier, &mut installes);
            let _ = installes.ecrire(&moi.donnees.join("variantes").join("mods.json"));
            r
        })
        .await
        .map_err(|_| "L'installation du mod a planté.".to_string())??;
        Ok(fichier.to_string_lossy().into_owned())
    }

    /// Les mods installés qui ont une nouvelle version au catalogue.
    pub fn mods_a_jour(&self) -> Vec<mods::ModRetro> {
        let cat = self.catalogue_mods.lock().unwrap();
        self.mods_installes.lock().unwrap().a_mettre_a_jour(&cat).into_iter().cloned().collect()
    }

    pub fn mod_retour_arriere(&self, id: &str) -> Result<String, String> {
        let mut installes = self.mods_installes.lock().unwrap();
        let r = mods::retour_arriere(id, &mut installes);
        let _ = installes.ecrire(&self.donnees.join("variantes").join("mods.json"));
        r
    }

    // ----- Outils -----

    pub fn diagnostic(&self, version_app: &str) -> String {
        diagnostic::rapport(&self.emplacements(), version_app, &self.erreurs.lock().unwrap())
    }

    pub fn exporter(&self, vers: &Path) -> Result<(), String> {
        if let Some(f) = &self.fichier {
            let _ = fs::copy(f, self.donnees.join("retro.json"));
        }
        let octets = sauvegarde::exporter(&self.donnees)?;
        fs::write(vers, octets).map_err(|e| e.to_string())
    }

    pub fn restaurer(&self, depuis: &Path) -> Result<usize, String> {
        let octets = fs::read(depuis).map_err(|e| e.to_string())?;
        let n = sauvegarde::restaurer(&octets, &self.donnees)?;
        if let Some(f) = &self.fichier {
            let copie = self.donnees.join("retro.json");
            if copie.is_file() {
                let _ = fs::copy(&copie, f);
                *self.reglages.lock().unwrap() = lire_json(f);
            }
        }
        *self.profils.lock().unwrap() = lire_json(&self.donnees.join("profils.json"));
        *self.historique.lock().unwrap() = Historique::charger(&self.donnees.join("historique.json"));
        Ok(n)
    }

    /// Compresse une image disque (RVZ ou CHD). L'original reste en place.
    pub fn convertir(&self, chemin: &str) -> Result<String, String> {
        let (console, _, v) = self.trouver(chemin)?;
        let ou = self.emplacements();
        let (commande, sortie) = lancement::conversion(console, &v.chemin, &ou)
            .ok_or("Ce fichier est déjà compressé ou ne se compresse pas.")?;
        if !commande.programme.is_file() {
            return Err("L'outil de compression n'est pas installé : lance l'installation des émulateurs.".into());
        }
        let statut = commande.vers_process().status().map_err(|e| e.to_string())?;
        if !statut.success() || !sortie.is_file() {
            return Err("La compression a échoué : l'original n'a pas été touché.".into());
        }
        Ok(sortie.to_string_lossy().into_owned())
    }

    // ----- Room et netplay -----

    fn place(&self, chemin: &str) -> Result<(Console, String, Place), String> {
        let (console, titre, v) = self.trouver(chemin)?;
        let sha1 = match v.empreintes {
            Some(e) => e.sha1,
            None => squad_retro::installation::sha256(&fs::read(&v.chemin).map_err(|e| e.to_string())?),
        };
        Ok((
            console,
            titre,
            Place { pseudo: self.pseudo(), pret: false, versions: Etat::charger(&self.emplacements()).versions(), sha1, avatar: Some(self.avatar()) },
        ))
    }

    /// Ouvre une room pour ce jeu. Retourne le lien d'invitation.
    pub fn heberger(self: &Arc<Self>, app: AppHandle, chemin: &str, max: usize, mot_de_passe: Option<String>, adresse: String) -> Result<String, String> {
        self.quitter_room();
        let (console, titre, place) = self.place(chemin)?;
        let port_jeu = match console.moteur() {
            squad_retro::Moteur::Dolphin => netplay::PORT_DOLPHIN,
            _ => netplay::PORT_RETROARCH,
        };
        let id = format!("{:x}", room_reseau::maintenant_ms());
        let room = Room::nouvelle(&id, console, &titre, place, max, mot_de_passe.is_some());
        let session = Session { role: Role::Heberger { port: port_jeu }, pseudo: self.pseudo(), mot_de_passe: mot_de_passe.clone(), relais: None };
        *self.prevue.lock().unwrap() = Some((chemin.to_string(), session));
        let moi = self.clone();
        let app2 = app.clone();
        let serveur = ServeurRoom::ouvrir(room, room_reseau::PORT_ROOM, mot_de_passe, move |room, dans| {
            moi.sur_room(&app2, room, dans);
        })?;
        serveur.diffuser();
        *self.serveur.lock().unwrap() = Some(serveur);
        Ok(Invitation { room: id, console, titre, adresse, port: room_reseau::PORT_ROOM, privee: self.prevue_privee() }.lien())
    }

    fn prevue_privee(&self) -> bool {
        self.prevue.lock().unwrap().as_ref().map(|(_, s)| s.mot_de_passe.is_some()).unwrap_or(false)
    }

    /// Rejoint une room avec un lien d'invitation, pour la version du jeu choisie.
    pub fn rejoindre(self: &Arc<Self>, app: AppHandle, lien: &str, chemin: &str, mot_de_passe: Option<String>) -> Result<(), String> {
        self.quitter_room();
        let inv = Invitation::lire(lien.trim())?;
        let (console, _, place) = self.place(chemin)?;
        if console != inv.console {
            return Err(format!("Cette room est pour la {}, pas la {}.", inv.console.nom(), console.nom()));
        }
        let port_jeu = match console.moteur() {
            squad_retro::Moteur::Dolphin => netplay::PORT_DOLPHIN,
            _ => netplay::PORT_RETROARCH,
        };
        let session = Session {
            role: Role::Rejoindre { adresse: inv.adresse.clone(), port: port_jeu },
            pseudo: self.pseudo(),
            mot_de_passe: mot_de_passe.clone(),
            relais: None,
        };
        *self.prevue.lock().unwrap() = Some((chemin.to_string(), session));
        let moi = self.clone();
        let client = ClientRoom::rejoindre(&inv.adresse, inv.port, place, mot_de_passe, move |n| match n {
            Nouvelle::Room { room, dans_ms } => moi.sur_room(&app, &room, dans_ms),
            Nouvelle::Refus { raison } => {
                let _ = app.emit("retro-room-refus", raison);
            }
        })?;
        *self.client.lock().unwrap() = Some(client);
        Ok(())
    }

    /// Chaque changement de la room : la fenêtre est prévenue, et le jeu se
    /// lance chez chacun à la fin du compte à rebours.
    fn sur_room(self: &Arc<Self>, app: &AppHandle, room: &Room, dans_ms: Option<u64>) {
        self.noter_croises(room);
        let _ = app.emit("retro-room", serde_json::json!({ "room": room, "dans_ms": dans_ms }));
        if let Some(ms) = dans_ms {
            let moi = self.clone();
            let app = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(ms));
                let prevue = moi.prevue.lock().unwrap().take();
                if let Some((chemin, session)) = prevue {
                    // Les invités partent une seconde après l'hôte, le temps qu'il écoute.
                    if matches!(session.role, Role::Rejoindre { .. }) {
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    if let Err(e) = moi.lancer(&chemin, Some(session), Some(app.clone())) {
                        let _ = app.emit("retro-room-refus", e);
                    }
                }
            });
        }
    }

    pub fn pret(&self) -> Result<(), String> {
        if let Some(s) = self.serveur.lock().unwrap().as_ref() {
            s.pret_hote();
            return Ok(());
        }
        match self.client.lock().unwrap().as_ref() {
            Some(c) => c.pret(),
            None => Err("Tu n'es dans aucune room.".into()),
        }
    }

    pub fn lancer_room(&self) -> Result<u64, String> {
        match self.serveur.lock().unwrap().as_ref() {
            Some(s) => s.lancer(),
            None => Err("Seul l'hôte lance la partie.".into()),
        }
    }

    pub fn quitter_room(&self) {
        if let Some(s) = self.serveur.lock().unwrap().take() {
            s.fermer();
        }
        if let Some(c) = self.client.lock().unwrap().take() {
            c.quitter();
        }
        *self.prevue.lock().unwrap() = None;
    }

    pub fn noter_manettes(&self, noms: Vec<String>) {
        *self.manettes.lock().unwrap() = noms;
    }
}

/// Les étapes de l'assistant, avec le profil actuel.
#[derive(Serialize)]
pub struct Assistant {
    pub etapes: Vec<Etape>,
    pub profil: Profil,
    pub mapping: manettes::Mapping,
}

pub fn assistant(retro: &Retro, console: Console) -> Assistant {
    let profil = retro.profil(console);
    let mapping = profil.mapping.clone().unwrap_or_else(|| manettes::Mapping::par_defaut(console));
    Assistant { etapes: manettes::etapes(console), profil, mapping }
}

/// Les adresses autorisées au téléchargement : le dépôt Squad-retro et les
/// sites officiels des émulateurs. Le reste est refusé.
fn adresse_autorisee(url: &str) -> bool {
    const SITES: [&str; 5] = [
        "https://raw.githubusercontent.com/Reskell/",
        "https://github.com/",
        "https://buildbot.libretro.com/",
        "https://dl.dolphin-emu.org/",
        "https://objects.githubusercontent.com/",
    ];
    SITES.iter().any(|s| url.starts_with(s))
}

/// Au-delà, on refuse (Dolphin et RetroArch font moins de 200 Mo).
const TAILLE_MAX: u64 = 600 * 1024 * 1024;

async fn telecharger(url: &str) -> Result<Vec<u8>, String> {
    if !adresse_autorisee(url) {
        return Err(format!("Adresse de téléchargement refusée : {url}"));
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(900))
        .user_agent("SQUAD-RETRO")
        .build()
        .map_err(|_| "Téléchargement impossible.".to_string())?;
    let mut reponse = client
        .get(url)
        .send()
        .await
        .map_err(|_| "Pas de connexion : réessaie dans un instant.".to_string())?;
    if !reponse.status().is_success() {
        return Err(format!("Téléchargement refusé ({}) : {url}", reponse.status()));
    }
    if reponse.content_length().is_some_and(|n| n > TAILLE_MAX) {
        return Err("Fichier trop gros.".into());
    }
    let mut octets = Vec::new();
    while let Some(m) = reponse.chunk().await.map_err(|_| "Téléchargement interrompu.".to_string())? {
        if octets.len() as u64 + m.len() as u64 > TAILLE_MAX {
            return Err("Fichier trop gros.".into());
        }
        octets.extend_from_slice(&m);
    }
    Ok(octets)
}

async fn telecharger_texte(url: &str) -> Result<String, String> {
    let octets = telecharger(url).await?;
    String::from_utf8(octets).map_err(|_| "Fichier illisible.".to_string())
}

/// Le fil qui écoute les manettes pour l'assistant : chaque bouton appuyé
/// ou relâché part vers la fenêtre (« retro-manette »).
pub fn demarrer_manettes(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let Ok(mut gilrs) = gilrs::Gilrs::new() else { return };
        let noter = |g: &gilrs::Gilrs, app: &AppHandle| {
            let noms = g.gamepads().map(|(_, p)| p.name().to_string()).collect();
            app.state::<Arc<Retro>>().noter_manettes(noms);
        };
        noter(&gilrs, &app);
        loop {
            while let Some(gilrs::Event { id, event, .. }) = gilrs.next_event() {
                let (bouton, appuye) = match event {
                    gilrs::EventType::ButtonPressed(b, _) => (b, true),
                    gilrs::EventType::ButtonReleased(b, _) => (b, false),
                    gilrs::EventType::Connected | gilrs::EventType::Disconnected => {
                        noter(&gilrs, &app);
                        let _ = app.emit("retro-manettes", ());
                        continue;
                    }
                    _ => continue,
                };
                if let Some(b) = bouton_pad(bouton) {
                    let _ = app.emit(
                        "retro-manette",
                        serde_json::json!({ "manette": usize::from(id), "bouton": b, "appuye": appuye }),
                    );
                }
            }
            std::thread::sleep(Duration::from_millis(8));
        }
    });
}

fn bouton_pad(b: gilrs::Button) -> Option<BoutonPad> {
    use gilrs::Button::*;
    Some(match b {
        South => BoutonPad::Sud,
        East => BoutonPad::Est,
        West => BoutonPad::Ouest,
        North => BoutonPad::Nord,
        LeftTrigger => BoutonPad::L1,
        RightTrigger => BoutonPad::R1,
        LeftTrigger2 => BoutonPad::L2,
        RightTrigger2 => BoutonPad::R2,
        LeftThumb => BoutonPad::L3,
        RightThumb => BoutonPad::R3,
        Start => BoutonPad::Start,
        Select => BoutonPad::Select,
        DPadUp => BoutonPad::Haut,
        DPadDown => BoutonPad::Bas,
        DPadLeft => BoutonPad::Gauche,
        DPadRight => BoutonPad::Droite,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ne_lance_que_les_jeux_scannes() {
        let dir = std::env::temp_dir().join(format!("squad-retro-test-{}", std::process::id()));
        let snes = dir.join("jeux").join("snes");
        fs::create_dir_all(&snes).unwrap();
        fs::write(snes.join("Jeu (France).sfc"), b"x").unwrap();

        let retro = Arc::new(Retro::charger(Some(dir.join("app"))));
        assert!(retro.lancer("C:/Windows/System32/calc.exe", None, None).is_err());

        retro.choisir_dossier_jeux(&dir.join("jeux"));
        let fiches = retro.scanner().unwrap();
        assert_eq!(fiches.len(), 1);
        assert_eq!(fiches[0].titre, "Jeu");

        // Trouvé par le scan, mais les émulateurs ne sont pas installés : message clair.
        let erreur = retro.lancer(&fiches[0].variantes[0].chemin, None, None).unwrap_err();
        assert!(erreur.contains("installation des émulateurs"));
        assert!(retro.diagnostic("test").contains("installation des émulateurs"));

        // Les réglages survivent à un redémarrage.
        retro.choisir_pseudo("Swift");
        let relu = Retro::charger(Some(dir.join("app")));
        assert!(relu.reglages().dossier_jeux.is_some());
        assert_eq!(relu.reglages().pseudo.as_deref(), Some("Swift"));
        // La jaquette d'un jeu SNES vient des miniatures libretro.
        assert!(fiches[0].jaquette.contains("Super%20Nintendo"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn profils_theme_et_plaza() {
        let dir = std::env::temp_dir().join(format!("squad-retro-profils-{}", std::process::id()));
        let retro = Retro::charger(Some(dir.clone()));
        let c = retro
            .enregistrer_compte(Compte { id: String::new(), pseudo: "Vesper".into(), avatar: Avatar { tenue: 4, ..Default::default() } })
            .unwrap();
        // Un nouveau profil devient le profil actif, et son pseudo le pseudo en ligne.
        assert_eq!(retro.etat().comptes.actif.as_deref(), Some(c.id.as_str()));
        assert_eq!(retro.reglages().pseudo.as_deref(), Some("Vesper"));
        assert_eq!(retro.avatar().tenue, 4);
        retro.choisir_theme(Some("gamecube".into()), true, false, true);
        retro.choisir_theme(Some("<script>".into()), true, false, true);
        assert_eq!(retro.reglages().theme, None);
        retro.choisir_theme(Some("wii".into()), true, true, false);

        let place = |p: &str| Place { pseudo: p.into(), pret: false, versions: Default::default(), sha1: String::new(), avatar: None };
        let room = Room {
            id: "r".into(),
            console: Console::Gamecube,
            titre: "Mario Party 6".into(),
            sha1: String::new(),
            hote: "Bront".into(),
            max: 4,
            privee: false,
            places: vec![place("Bront"), place("Vesper")],
            etat: netplay::Etat::Attente,
        };
        retro.noter_croises(&room);
        let croises = retro.croises();
        assert_eq!(croises.len(), 1);
        assert_eq!(croises[0].pseudo, "Bront");

        let relu = Retro::charger(Some(dir.clone()));
        assert_eq!(relu.etat().comptes.liste.len(), 1);
        assert_eq!(relu.reglages().theme.as_deref(), Some("wii"));
        assert!(relu.reglages().muet);
        assert_eq!(relu.croises().len(), 1);
        relu.supprimer_compte(&c.id);
        assert!(relu.etat().comptes.liste.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn adresses() {
        assert!(adresse_autorisee("https://buildbot.libretro.com/stable/x.7z"));
        assert!(!adresse_autorisee("https://exemple.com/virus.exe"));
        assert!(!adresse_autorisee("http://buildbot.libretro.com/x"));
    }
}
