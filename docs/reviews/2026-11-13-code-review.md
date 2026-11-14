# Compte rendu — revue de code Bourso CLI/API

Date : 2026-11-13. Candidat : `b0fd81fcc5c473375efd1737f2b1c92377747884` (main, propre lors de la revue).

## Objectif et portée
CLI bancaire locale : comptes, export, quotes, virements, ordres et DCA. Quatre sous-agents, paramètre `model=gpt-6-astra` demandé pour chacun; revue statique collective de toutes les sources et configurations. Les identifiants de délégation ne sont pas une attestation du backend réellement utilisé.

## Synthèse
Utilisation financière automatisée déconseillée avant sécurisation des intentions, confirmations, résultats inconnus et DCA. Aucun P0 ni incident bancaire réel établi. Les constats source sont distingués des hypothèses; un scénario synthétique ne démontre pas son occurrence chez le fournisseur.

## Vérifications
- Arbre Git propre au début et après la revue; aucune modification applicative.
- Syntaxe shell : PASS; métadonnées Cargo racine offline/locked : PASS.
- Formatage : FAIL (huit fichiers).
- 13 tests unitaires API inventoriés; aucun test CLI/intégration repéré.
- Build/tests Rust/Clippy/couverture : NOT RUN, cache de dépendances incomplet.
- Réseau bancaire, audit d’advisories, compatibilité réelle et CI/release distante : NOT RUN.
- Probes Python en mémoire : certains prédicats/CSV reproduits, pas une exécution Rust.

## Constats et hypothèses

### R01 — P1 — Respecter le sens et la quantité explicitement demandés

**Références :** `src/bourso_api/src/client/trade/order.rs:45-70`; `src/lib.rs:351-360`

**Observation :** Le préremplissage est conservé lorsque sens et quantité sont déjà présents.

**Impact et limites :** Un préremplissage contradictoire peut produire une opération différente et un journal trompeur. Réponses bancaires actuelles non capturées.

**Correction proposée :** Donner priorité à l’intention explicite ou refuser les conflits; vérifier le payload réel.

**Preuve :** source; exécution Rust NOT RUN.

### R02 — P1 — Contrôler les résultats métier avant et après confirmation d’ordre

**Références :** `src/bourso_api/src/client/trade/order.rs:85-89,152-219`

**Observation :** Le résultat de check est ignoré; le statut métier de confirm n’est pas exposé.

**Impact et limites :** Confirmation sans garde cliente sur les messages et paramètres; dépôt et exécution peuvent être confondus. Acceptation serveur inconnue.

**Correction proposée :** Définir les états admissibles, comparer l’intention et retourner un état métier typé.

**Preuve :** source; exécution Rust NOT RUN.

### R03 — P1 — Prouver la réussite métier d’un virement

**Références :** `src/bourso_api/src/client/transfer/mod.rs:139-146,178-185,212-219,284-291,322-334`

**Observation :** HTTP 200 puis présence du mot Confirmation suffisent; corps intermédiaires ignorés.

**Impact et limites :** Faux succès/faux échec possibles; aucun incident bancaire observé.

**Correction proposée :** Valider transitions, reçu, référence et récapitulatif; résultat inconnu en l’absence de preuve.

**Preuve :** source; exécution Rust NOT RUN.

### R04 — P1 — Distinguer les résultats financiers inconnus des échecs certains

**Références :** `src/bourso_api/src/client/trade/order.rs:191-217`; `src/bourso_api/src/client/transfer/mod.rs:312-333`

**Observation :** Une perte de réponse après POST final retourne une erreur ordinaire.

**Impact et limites :** Une relance peut dupliquer une opération; pas de retry automatique ni déduplication serveur établie.

**Correction proposée :** Exposer OutcomeUnknown, conserver les références disponibles et imposer rapprochement avant reprise.

**Preuve :** source; exécution Rust NOT RUN.

### R05 — P1 — Supprimer les secrets des journaux et protéger les fichiers locaux

**Références :** `src/settings.rs:89-94,114-128`; `src/bourso_api/src/client/mod.rs:340-345,453,551-604`; `src/bourso_api/src/client/config.rs:75-84`; `src/bourso_api/src/client/virtual_pad.rs:50-55`

**Observation :** Le fichier capture TRACE et reçoit tokens, URL sensibles, HTML/config bruts et caractères de mot de passe invalides.

**Impact et limites :** Persistance sensible et partage involontaire possibles; droits réels dépendants de l’environnement, aucun rejeu de token démontré.

**Correction proposée :** Expurger à la source; permissions privées et politique de rétention explicites.

**Preuve :** source; exécution Rust NOT RUN.

### R06 — P1 — Rendre le suivi DCA cohérent avec le résultat de l’opération

**Références :** `bourso-launchd.sh:14-40`; `README.md:168-169`

**Observation :** Répertoire absent, timestamp après lancement asynchrone, aucun verrou.

**Impact et limites :** Répétition possible, investissement sauté après échec et lancements concurrents.

**Correction proposée :** État tentative/succès/inconnu, verrou, résultat observable et reprise prudente.

**Preuve :** source; exécution Rust NOT RUN.

### R07 — P1 — Refuser les exports invalides sans écraser les données précédentes

**Références :** `src/bourso_api/src/client/transaction.rs:90-125,181-206`; `src/lib.rs:287-321`

**Observation :** Statut HTTP non validé; formulaire assimilé à vide; schéma CSV non contrôlé.

**Impact et limites :** Erreur masquée en zéro transaction et remplacement du fichier utilisateur.

**Correction proposée :** Valider HTTP/session/schéma et préserver la destination en cas d’erreur.

**Preuve :** source; exécution Rust NOT RUN.

### R08 — P2 — Préserver la date d’expiration explicitement fournie

**Références :** `src/bourso_api/src/client/trade/order.rs:72-79`

**Observation :** Une date présente est remplacée par la date UTC du jour.

**Impact et limites :** Validité raccourcie involontairement.

**Correction proposée :** Préserver la valeur explicite; appliquer un défaut seulement si absente.

**Preuve :** source; exécution Rust NOT RUN.

### R09 — P2 — Valider les montants avant toute authentification

**Références :** `src/main.rs:221-226`; `src/lib.rs:380-383`; `src/bourso_api/src/client/transfer/mod.rs:195-203,359-362`

**Observation :** Texte invalide panique tardivement; NaN/infini passent le seuil; arrondi implicite.

**Impact et limites :** MFA inutile, payload invalide et montant affiché différent du montant soumis.

**Correction proposée :** Parser au point d’entrée; centimes ou décimal, bornes et politique de précision explicites.

**Preuve :** source; exécution Rust NOT RUN.

### R10 — P2 — Parser les montants bancaires sans corruption silencieuse

**Références :** `src/bourso_api/src/client/transaction.rs:160-169,193-203`; `src/bourso_api/src/client/account.rs:82-92`; `src/bourso_api/src/constants.rs:6`

**Observation :** Montant invalide devient zéro; certains espaces Unicode restent; soldes 12 ou 12,3 mal normalisés.

**Impact et limites :** Export/solde faux ou panic pour des formats acceptés par le parseur actuel. Variantes réelles à confirmer.

**Correction proposée :** Parseur commun fallible, normalisation contrôlée et représentation monétaire exacte.

**Preuve :** source; exécution Rust NOT RUN.

### R11 — P2 — Ne pas masquer les erreurs de récupération des comptes

**Références :** `src/bourso_api/src/client/account.rs:24-98`

**Observation :** Les erreurs de catégories sont remplacées par des vecteurs vides.

**Impact et limites :** Inventaire incomplet ou vide affiché comme succès.

**Correction proposée :** Distinguer absence légitime, session expirée et HTML incompatible.

**Preuve :** source; exécution Rust NOT RUN.

### R12 — P2 — Produire un CSV de sortie correctement échappé

**Références :** `src/lib.rs:296-315`

**Observation :** Les champs sont joints avec ; sans échappement.

**Impact et limites :** Libellés avec séparateur/retour ligne corrompent les colonnes.

**Correction proposée :** Utiliser un writer CSV et un test aller-retour à onze champs.

**Preuve :** source; exécution Rust NOT RUN.

### R13 — P2 — Garantir la deadline MFA dans toutes les branches

**Références :** `src/lib.rs:169-220`; `src/bourso_api/src/client/mod.rs:111-115,518-524`

**Observation :** La branche QR ne contrôle pas le maximum; temps réseau non compté; pas de timeout explicite.

**Impact et limites :** Attente au-delà des cinq minutes annoncées, éventuellement indéfinie.

**Correction proposée :** Deadline monotone incluant HTTP et pauses, délais explicites sans retry financier aveugle.

**Preuve :** source; exécution Rust NOT RUN.

### R14 — P2 — Transformer les entrées d’authentification malformées en erreurs

**Références :** `src/bourso_api/src/client/config.rs:76-81`; `src/bourso_api/src/client/mod.rs:296,367,393,424,452,504`; `src/bourso_api/src/client/virtual_pad.rs:96-104`

**Observation :** Des unwrap dépendent de réponses externes ou d’une séquence init/login.

**Impact et limites :** Crash sur changement HTML, null/champ absent ou mauvais état.

**Correction proposée :** Erreurs typées expurgées et état de session explicite.

**Preuve :** source; exécution Rust NOT RUN.

### R15 — P2 — Refuser un clavier virtuel incomplet ou ambigu

**Références :** `src/bourso_api/src/client/virtual_pad.rs:71-84`; `src/bourso_api/src/client/mod.rs:223-227,247-260`

**Observation :** Dix chaînes vides sont initialisées, sans vérification finale de complétude/unicité.

**Impact et limites :** Mauvais mot de passe encodé et échecs de connexion répétés; verrouillage bancaire non prouvé.

**Correction proposée :** Vérifier bijection des dix chiffres et clés non vides avant login.

**Preuve :** source; exécution Rust NOT RUN.

### R16 — P2 — Retourner un échec système quand la configuration manque

**Références :** `src/lib.rs:127-129`; `src/main.rs:243-245`

**Observation :** Les commandes bancaires sans identifiant retournent Ok après un avertissement.

**Impact et limites :** Automatisation croit que l’opération a réussi.

**Correction proposée :** Erreur contextualisée et code de sortie non nul avant tout appel bancaire.

**Preuve :** source; exécution Rust NOT RUN.

### R17 — P2 — Permettre la réparation de config et les quotes sans config valide

**Références :** `src/lib.rs:23-46`; `src/settings.rs:41-61`

**Observation :** Chargement JSON avant dispatch, y compris config et quote.

**Impact et limites :** Un JSON tronqué bloque sa réparation et les cotations publiques.

**Correction proposée :** Chargement conditionnel et distinction absence/permission/corruption.

**Preuve :** source; exécution Rust NOT RUN.

### R18 — P2 — Corriger la sémantique des filtres de comptes

**Références :** `src/main.rs:30-51`; `src/lib.rs:242-253`

**Observation :** Options sans SetTrue; valeurs inutilisées et priorité silencieuse entre filtres.

**Impact et limites :** accounts --trading exige une valeur, combinaisons ambiguës. Source uniquement, Clap verrouillé non exécuté.

**Correction proposée :** Drapeaux booléens avec combinaison ou exclusion explicite.

**Preuve :** source; exécution Rust NOT RUN.

### R19 — P2 — Gérer explicitement les historiques de cotation vides

**Références :** `src/lib.rs:88-95`; `src/bourso_api/src/client/trade/tick.rs:90-108`

**Observation :** Dernier cours unwrap; moyenne NaN, minimum MAX et maximum zéro sur série vide.

**Impact et limites :** Crash ou cours artificiels.

**Correction proposée :** Agrégats optionnels/fallibles et erreur aucune cotation.

**Preuve :** source; exécution Rust NOT RUN.

### R20 — P2 — Évaluer l’ouverture selon le marché et non le poste local

**Références :** `src/bourso_api/src/client/trade/feed.rs:15-23`

**Observation :** Comparaison des heures Local sans fuseau, date ni calendrier.

**Impact et limites :** Week-end ouvert, désaccord entre fuseaux et séance nocturne mal traitée; aucun caller CLI actuel.

**Correction proposée :** Statut fournisseur ou calendrier/fuseau explicite et horloge injectée.

**Preuve :** source; exécution Rust NOT RUN.

### R21 — P2 — Limiter les redirections d’export aux origines autorisées

**Références :** `src/bourso_api/src/client/transaction.rs:68-85`

**Observation :** URL absolue commençant par http suivie sans vérifier schéma/hôte.

**Impact et limites :** Requête hors banque/en clair possible sur redirection; contrôle attaquant et fuite de cookies non démontrés.

**Correction proposée :** Résolution URL standard, HTTPS et allowlist d’origines.

**Preuve :** source; exécution Rust NOT RUN.

### R22 — P3 — Aligner la limite du motif sur le contrat Unicode

**Références :** `src/bourso_api/src/client/transfer/mod.rs:377-380`; `src/bourso_api/src/client/transfer/error.rs:24-25`

**Observation :** len compte les octets alors que le message annonce 50 caractères.

**Impact et limites :** 26 é sont refusés.

**Correction proposée :** Confirmer l’unité fournisseur et appliquer/documenter la bonne limite.

**Preuve :** source; exécution Rust NOT RUN.

### R23 — P1 — Mesurer la couverture sans amputer l’API livrée

**Références :** `src/bourso_api/src/client/mod.rs:132-133`; `src/bourso_api/src/client/account.rs:24-31`; `.github/workflows/coverage.yml:15-17`

**Observation :** get_headers retiré sous tarpaulin_include reste appelé; 36 exclusions dont parcours financiers.

**Impact et limites :** Incohérence de compilation conditionnelle et couverture non représentative. Workflow distant non exécuté.

**Correction proposée :** Compiler/instrumenter la même API; tester les parcours avec transport simulé.

**Preuve :** source; exécution Rust NOT RUN.

### R24 — P1 — Vérifier les deux crates avant intégration et publication

**Références :** `.github/workflows/cicd.yml:4-7,47-54`; `.github/workflows/coverage.yml:3,17,22`

**Observation :** Pas de trigger pull_request; tests complets au tag; aucun test CLI/intégration.

**Impact et limites :** Régressions monétaires sans garde CI déclarée avant intégration; protections externes inconnues.

**Correction proposée :** CI PR, tests deux crates, fmt/clippy et scénarios financiers hermétiques.

**Preuve :** source; exécution Rust NOT RUN.

### R25 — P2 — Unifier les résolutions de dépendances et les builds verrouillés

**Références :** `Cargo.toml`; `Cargo.lock`; `src/bourso_api/Cargo.toml`; `src/bourso_api/Cargo.lock:162-179`

**Observation :** Verrou API encore 0.4.0, dépendances directes périmées et résolutions divergentes; pas de workspace.

**Impact et limites :** Build autonome/racine non équivalents; mise à jour implicite possible.

**Correction proposée :** Workspace à verrou unique ou politique autonome explicite; --locked.

**Preuve :** source; exécution Rust NOT RUN.

### R26 — P2 — Déclarer et vérifier un MSRV réellement supporté

**Références :** `README.md:67-71`; `rust-toolchain.toml`; `Cargo.lock:863-872`; `src/qrcode.rs:1-26`

**Observation :** Rust >=1.77.2 annoncé mais image 0.25.9 déclare 1.85 dans l’index local; features QR superflues.

**Impact et limites :** Compilation impossible sur minimum annoncé; outil stable flottant.

**Correction proposée :** MSRV réel, toolchain testée et features nécessaires seulement.

**Preuve :** source; exécution Rust NOT RUN.

### R27 — P2 — Réduire les privilèges et figer la chaîne de distribution

**Références :** `.github/workflows/cicd.yml:9-10,30,39,45,89`; `.github/workflows/coverage.yml:8-23`; `.devcontainer/devcontainer.json:7,35-37`

**Observation :** Actions par tags, images flottantes, compilation avec contents write et couverture sans seccomp.

**Impact et limites :** Risque supply chain conditionnel; aucune compromission ou CVE démontrée.

**Correction proposée :** SHA/digests, permissions par job, checkout sans credentials persistants, exceptions SLSA documentées.

**Preuve :** source; exécution Rust NOT RUN.

### R28 — P2 — Documenter honnêtement la configuration et le DCA

**Références :** `README.md:73,79-85,144-212`; `src/main.rs:57-61`; `src/settings.rs:7-12,67-77`

**Observation :** config exige username absent des exemples; credentials JSON peut contenir password; DCA interactif; Windows -NoExit; chown 777 incorrect.

**Impact et limites :** Parcours de démarrage cassé et promesses sécurité/autonomie trompeuses.

**Correction proposée :** Exemples exécutables, modes de secrets/interaction explicites, aucun contournement MFA.

**Preuve :** source; exécution Rust NOT RUN.

### R29 — P3 — Distribuer les notices de licence et corriger le formatage

**Références :** `LICENSE:12-13`; `.github/workflows/cicd.yml:60-96`; `Cargo.toml`; `src/bourso_api/Cargo.toml`

**Observation :** Archives décrites ne contiennent que l’exécutable; manifestes sans licence; fmt --check échoue sur huit fichiers.

**Impact et limites :** Distribution sans notice et gate de formatage en échec.

**Correction proposée :** Inclure notices nécessaires, métadonnées licence et formatage vérifié séparément.

**Preuve :** source; exécution Rust NOT RUN.

### H01 — P1 — Établir les contrats fournisseur avant de changer les sémantiques financières

**Références :** `src/bourso_api/src/client/trade/order.rs:58-66,229-255`; `src/bourso_api/src/client/transfer/mod.rs`

**Observation :** Montant total vs prix unitaire, statuts check/confirm, annulation, étapes et Unicode non documentés dans les preuves.

**Impact et limites :** Hypothèses seulement; éviter une correction spéculative pouvant changer les fonds.

**Correction proposée :** Fixtures anonymisées/provenance, contrats et décisions; pas de connexion bancaire sans autorisation dédiée.

**Preuve :** hypothesis; exécution Rust NOT RUN.

## Politique de planification et de validation
Beads est la source de vérité des tâches, critères, statuts et dépendances. Le JSON voisin sérialise uniquement ce CR, pas un deuxième tracker. Lire le ticket et son champ acceptance pour les scénarios de régression détaillés.

Objectifs proposés : intention financière respectée, résultats fiables, aucun secret dans les sinks, exports intègres, erreurs sans panic, CI locale/simulée représentative. Non-objectifs : modifier la stratégie d’investissement, contourner MFA, ajouter une nouvelle authentification, accéder à la banque, installer des dépendances ou publier sans permission spécifique.

Critères des tickets proposés pour la correction, non encore vérifiés. NFR : zéro secret sentinelle dans les logs de tests; permissions Unix privées testées (0700/0600 proposées); deadline MFA de 300 s issue du comportement annoncé, horloge simulée couvrant toutes branches; montant exact sans arrondi implicite sur corpus de frontières. Autres objectifs de performance : non définis, pas de chiffres inventés.

Validation indépendante du candidat figé et de ses preuves requise pour les corrections financières/sécurité et l’acceptation globale. Rubrique slice-acceptance v1.1.0, mode small-task pour tickets bornés, weighted pour acceptation globale; aucun score calculé. Capturer sources suivies/non suivies et environnement avant validation, exclure explicitement les sorties de build; ne pas réutiliser des preuves après changement de candidat. Trois boucles correctives maximum avant arrêt et décision.

## Persistance et effet de l’initialisation
La demande autorise la sauvegarde du CR et la planification, pas l’implémentation. `bd init --non-interactive --skip-agents --skip-hooks` a malgré ces options créé automatiquement le commit local `e8dd1e6034a7350a15de3eb324593bd3169bf292` (configuration Beads et .gitignore) et configuré un remote Dolt. Effet inattendu signalé; aucun hook installé, aucun push/sync effectué. Code applicatif inchangé entre ce commit et le candidat audité. Aucun rollback Git implicite.

## Référence du plan
Epic Beads : `bourso-y3u`. Les références Beads par constat sont dans le JSON du CR. Statuts, critères et dépendances restent canoniques dans Beads; consulter `bd show bourso-y3u` et `bd ready`. Aucun enfant fermé ni assigné lors de la planification.
