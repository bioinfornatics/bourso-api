# Recherche publique fournisseur — .4 / .6 / .7 / .24 / .26

## Portée et décision

Recherche documentaire **READ ONLY**, réalisée le **14 novembre 2026**, date d'accès constatée sur l'horloge d'exécution (premier relevé : `2026-11-14T00:49:53+01:00`). Cette date n'est ni une date de publication ni une date de mise à jour des sources. Aucune date éditoriale vérifiée ; actualité contractuelle non garantie.

Les identifiants .4/.6/.7/.24/.26 sont ceux du mandat. Leur libellé exact n'a pas été fourni ; aucun ticket, code ou document bancaire local n'a été lu pour en deviner l'association. La matrice ci-dessous est donc **thématique**, applicable à ce lot, et non une attribution inventée de critères aux numéros.

**Résultat : éléments métier partiellement documentés ; aucun contrat API établi et aucune clôture recommandée.** Une FAQ d'interface ne permet pas de spécifier `ordStat`, `resourceId`, une énumération transport, des champs HTML ou les séquences techniques `prepare/check/confirm`.

## Méthode et limites de collecte

- Requêtes publiques GET via `curl -q`, sans authentification, cookies fournis, session persistante, suivi automatique des redirections ni soumission de formulaire. Aucun endpoint transactionnel interrogé.
- `robots.txt` consultés avant les contenus : `https://www.boursobank.com/robots.txt`, `https://www.boursorama.com/robots.txt`, `https://html.duckduckgo.com/robots.txt`. Les chemins d'aide et de navigation retenus ne sont pas interdits par les règles observées. Les chemins `feature-redirect` et les accès client n'ont pas été suivis.
- DuckDuckGo HTML a présenté un challenge à la première recherche ; arrêt immédiat, sans contournement ni nouvelle recherche sur cet hôte. Aucun résultat moteur utilisé comme preuve.
- Les FAQ ont été découvertes par les index publics de l'aide. Le guide PDF était référencé derrière une redirection authentifiée : ni cette redirection ni sa cible extraite n'ont été interrogées.
- « Marchés et horaires » a été découvert **indépendamment** via un lien public dans `https://www.boursorama.com/bourse/`, puis ouvert directement. Aucun contournement du lien client.
- Les pages retenues ci-dessous ont répondu HTTP 200, sans besoin d'authentification pour leur texte principal. Les blocs de connexion aux listes personnelles n'ont pas été utilisés.
- Pas d'installation, pas de commande Git/Beads, pas de lecture de documents personnels, pas de transaction bancaire. Seuls cette synthèse et ses métadonnées publiques sont écrits dans le répertoire autorisé. Les pages intégrales ne sont pas archivées.
- Recherche bornée : jamais plus de trois sondes consécutives sans progrès par piste. Les résultats négatifs signifient **non trouvé dans cet échantillon**, jamais absence générale de documentation.

## Critère → preuve → inconnu

| Critère thématique | Preuve publique observée | Ce qui reste inconnu / réserve |
|---|---|---|
| Types d'ordre | S1 renvoie au guide et limite le seuil à Paris/Amsterdam/Bruxelles ; S2 énumère marché, limité, seuil et plage pour Euronext. | **Divergence de périmètre** : S2 inclut Lisbonne dans son énumération générale, S1 l'omet pour le seuil. Ne pas résoudre par supposition. Codes API, combinaisons permises par instrument/compte et exhaustivité inconnus. |
| Validité | S2 décrit une validité jour pour les marchés européens hors Euronext ; S4 définit l'expiration. | Énumérations et formats API, limites de dates, cutoff, fuseau, jours non ouvrés, GTC et couverture hors de ce périmètre non établis. |
| Consultation / annulation / statuts | S3 décrit « Mes ordres » et une annulation avec confirmation dans l'interface ; S4 parle d'expiration et d'annulation. | Pas de machine d'états, ni transition garantie, ni état partiellement exécuté, ni correspondance `ordStat`. Le bouton de confirmation n'établit aucun protocole réseau. |
| Avis d'opéré | S5 décrit un avis par opération, ses informations et la disponibilité dès le premier jour ouvré suivant. | Aucun avis personnel consulté. Format réel, identifiants, `resourceId`, endpoint, jointure ordre/exécution/document et garanties d'unicité API inconnus. |
| Virement : motif / limite / unité | S6-S7 documentent un **plafond monétaire**, 15 000 € par jour et par compte par défaut. | Ce montant **n'est pas une limite du motif**. Longueur du motif, caractère vs octet, encodage, normalisation, caractères admis et sémantique API inconnus. Ne pas importer une limite SEPA générique. |
| Parcours de virement | S8 décrit une sélection du mode classique dans l'application ; le mode par défaut annoncé est instantané. | Aucun contrat HTML/API `prepare/check/confirm`, ni preuve d'idempotence, de validation ou de réussite d'un transfert. Aucun parcours exécuté. |
| Marché : horaires / fuseau | S9 donne des plages TAL ; S10 indique des horaires par place, certaines lignes explicitant « heure française ». | Pas d'identifiant IANA établi, d'offset UTC garanti, de règle DST, de gestion des semaines de décalage US/Europe, de statut ouvert/fermé en temps réel ni de calendrier de jours fériés complet. |
| Calendrier | S11 est un agenda macroéconomique et de sociétés cotées. | Ce n'est pas la preuve d'un calendrier des séances, fermetures, demi-séances ou interruptions. Une absence d'événement n'implique pas une fermeture de marché. |

## Sources primaires et courtes citations

Toutes les sources S1–S11 ont été consultées le **2026-11-14**, date de publication/mise à jour **non vérifiée**. Les titres ci-dessous correspondent aux titres de page observés, sans le suffixe du site. Les citations sont de courts extraits ; les autres formulations de cette synthèse sont des paraphrases.

### S1 — Quels sont les différents types d'ordres ?
- Éditeur : BoursoBank, aide officielle.
- URL : https://www.boursobank.com/aide-en-ligne/bourse/comment-gerer-mon-compte-bourse/passage-d-ordre/question/quels-sont-les-differents-types-d-ordres-161
- Citation : « disponible uniquement pour les marché Euronext Paris, Euronext Amsterdam et Euronext Bruxelles ».
- Portée : seuil de déclenchement. Le guide lié n'a pas été consulté.

### S2 — Quelles sont les conditions de passage d'ordre sur les marchés européens ?
- Éditeur : BoursoBank, aide officielle.
- URL : https://www.boursobank.com/aide-en-ligne/bourse/comment-gerer-mon-compte-bourse/passage-d-ordre/question/quelles-sont-les-conditions-de-passage-d-ordre-sur-les-marches-europeens-70285167
- Citation : « Ordre limité avec une validité jour uniquement ».
- Portée : cette restriction concerne les marchés européens **hors Euronext**. La page énumère séparément quatre types pour Euronext (Paris, Amsterdam, Bruxelles, Lisbonne). Divergence avec S1 à faire clarifier, pas à masquer.

### S3 — Comment passer, consulter ou annuler un ordre en bourse ?
- Éditeur : BoursoBank, aide officielle.
- URL : https://www.boursobank.com/aide-en-ligne/bourse/comment-gerer-mon-compte-bourse/passage-d-ordre/question/comment-passer-consulter-ou-annuler-un-ordre-en-bourse-198
- Citation : « cliquez sur "Annuler" et enfin "Confirmer" ».
- Portée : instruction d'interface, pas description d'un échange HTTP. La FAQ indique que seule l'exécution est facturée.

### S4 — Y a-t-il des frais lorsqu'un ordre de Bourse tombe en fin de validité ?
- Éditeur : BoursoBank, aide officielle.
- URL : https://www.boursobank.com/aide-en-ligne/bourse/comment-gerer-mon-compte-bourse/passage-d-ordre/question/y-a-t-il-des-frais-lorsqu-un-ordre-de-bourse-tombe-en-fin-de-validite-25269483
- Citation : « Tombé en fin de validité signifie que l'ordre est arrivé à expiration ».
- Portée : définition métier ; aucun code de statut transport.

### S5 — Qu'est-ce qu'un avis d'opéré ?
- Éditeur : BoursoBank, aide officielle.
- URL : https://www.boursobank.com/aide-en-ligne/bourse/comment-gerer-mon-compte-bourse/mes-documents-et-releves-bourse/question/qu-est-ce-qu-un-avis-d-opere-159
- Citations : « Il est édité un avis par opération. » ; « dès le premier jour ouvré suivant l'opération ».
- Paraphrase : l'avis contient date, valeur, sens, type de marché, cours, place de négociation et frais, dont éventuellement le taux de change. La consultation s'effectue dans les documents Bourse ; elle n'a pas été réalisée.

### S6 — Quel est le montant maximum pour un virement ?
- Éditeur : BoursoBank, aide officielle.
- URL : https://www.boursobank.com/aide-en-ligne/banque/virement/generalites/question/quel-est-le-montant-maximum-pour-un-virement-294
- Citation : « Par défaut, chaque compte bancaire dispose d’un plafond de virement de 15 000 €. »
- Portée : défaut modifiable, pas plafond absolu universel ni propriété du compte d'un utilisateur.

### S7 — Comment fonctionne le plafond de virement ?
- Éditeur : BoursoBank, aide officielle.
- URL : https://www.boursobank.com/aide-en-ligne/banque/virement/generalites/question/comment-fonctionne-le-plafond-de-virement-57927627
- Citation : « Les virements permanents programmés n'impactent pas le plafond de virement de votre compte ».
- Paraphrase : la page précise 15 000 € **par jour** par défaut, appliqués séparément à chaque compte. Ce n'est pas une spécification de remise à zéro, de timezone, de montant API en euros/centimes ou de motif.

### S8 — Comment effectuer un Virement classique (SEPA) ?
- Éditeur : BoursoBank, aide officielle.
- URL : https://www.boursobank.com/aide-en-ligne/banque/virement/virement-sepa-classique/question/comment-effectuer-un-virement-classique-sepa-73063596
- Citation : « Par défaut, votre virement est réalisé en virement instantané ».
- Portée : description de l'application ; aucune action entreprise.

### S9 — Comment s'organise une séance boursière et où trouver les horaires des différents marchés ?
- Éditeur : BoursoBank, aide officielle.
- URL : https://www.boursobank.com/aide-en-ligne/bourse/comment-investir-en-bourse/fonctionnement-de-la-bourse/question/comment-s-organise-une-seance-boursiere-et-ou-trouver-les-horaires-des-differents-marches-178
- Citation : « Valeurs cotées en continu : 17h35 à 17h40 ».
- Portée : plage de disponibilité de l'ordre TAL, pas horaires complets de toutes les bourses. Le texte précise aussi le double fixing ; aucune timezone technique fournie.

### S10 — Horaires d'ouverture et de fermeture des bourses mondiales
- Éditeur : Boursorama, portail officiel présentant la politique d'exécution BoursoBank.
- URL : https://www.boursorama.com/bourse/politique-execution/
- Citations : « 15H30 - 22H00 (heure française) » ; « Cette liste ne constitue en aucun cas un engagement de BoursoBank. »
- Paraphrase : première citation associée à la ligne NYSE Composite / NASDAQ Composite, lieu d'exécution indiqué NYSE. La page donne également 9H00–17H30, fixing 17H35 et TAL 17H35–17H40 pour le CAC 40 sur Euronext Paris. Ne pas généraliser à tous les produits ou tous les jours : la source avertit que la liste est informative et susceptible d'évolution.

### S11 — Calendrier bourse et économie
- Éditeur / hébergeur : Boursorama ; la page attribue des contenus à CercleFinance.com et Cofisem.com.
- URL : https://www.boursorama.com/bourse/actualites/calendriers/
- Citations : « Macroéconomique » ; « Sociétés cotées ».
- Portée : catégories d'événements effectivement observées. Aucun événement ni date affichée dans cet agenda n'est repris comme règle de séance.

## Pistes arrêtées et vérifications encore nécessaires

- **Motif de virement** : index Généralités et SEPA consultés ; après la FAQ du parcours classique, les index Instantané puis Hors zone SEPA n'ont pas apporté de limite du motif. Arrêt au troisième sondage consécutif sans progrès sur cette question. Aucune longueur inventée.
- **Statuts d'ordre** : les FAQ consultation/annulation, expiration et affichage des positions ne fournissent pas une énumération technique. L'affichage « Position instantanée » n'est pas un statut d'ordre.
- **Calendrier** : horaires publics trouvés, mais l'agenda trouvé ne constitue pas un calendrier de marché. Pas de déduction par calcul de weekday ni de statut instantané.
- **Guide** : cible uniquement rencontrée dans des liens de redirection client authentifiée ; non consultée. Aucun PDF bancaire local recherché.
- **Contrats** : obtenir ultérieurement une documentation fournisseur explicite et autorisée pour codes/types/statuts/validité, formats et identifiants d'avis, validation du motif (unité comprise), schémas et transitions de virement, fuseaux et calendrier de séances. Cette recherche ne donne aucune autorisation de lever ses limites de sécurité pour combler les inconnus.
