# Sécurité

Komago ouvre des fichiers venus de partout : archives, images, métadonnées. La sécurité de cette lecture est une priorité ; les protections en place sont décrites dans [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#sécurité-de-limport).

## Signaler une faille

**N'ouvrez pas d'issue publique.** Utilisez le signalement privé de GitHub : onglet **Security**, puis **Report a vulnerability** ([lien direct](https://github.com/kara0905/Komago/security/advisories/new)).

Indiquez si possible :
- la version de Komago et votre système ;
- comment reproduire le problème, avec un fichier d'exemple fabriqué pour l'occasion (jamais une œuvre protégée) ;
- l'impact que vous envisagez.

Vous recevrez un premier retour dans les sept jours. Une fois la correction publiée, la faille sera décrite dans les notes de version et vous serez remercié, si vous le souhaitez.

## Versions suivies

Seule la dernière version publiée reçoit des correctifs de sécurité.

## Limites connues

Elles sont documentées publiquement, car elles ne constituent pas des failles inconnues :
- la bibliothèque UnRAR décompresse une entrée de RAR en mémoire avant que Komago puisse en plafonner la taille ;
- les paquets publiés ne sont pas signés.
