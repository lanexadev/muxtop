# muxtop

**A modern, multiplexed system monitor for the terminal.**

[![CI](https://github.com/lanexadev/muxtop/actions/workflows/ci.yml/badge.svg)](https://github.com/lanexadev/muxtop/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/muxtop.svg)](https://crates.io/crates/muxtop)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE-MIT)

muxtop remplace le workflow `htop` + `iftop` + `ctop` par une interface à onglets unique.
Pensez htop, mais avec une UX de multiplexeur (à la tmux/zellij) et une palette de commandes à la VS Code.

---

## Installation

### Via crates.io

```sh
cargo install muxtop
```

### Via Homebrew (macOS / Linux)

```sh
brew tap lanexadev/tap
brew install muxtop
```

### Via APT (Debian / Ubuntu)

```sh
# Ajout du repo (une seule fois)
curl -fsSL https://lanexadev.github.io/apt/gpg.key | sudo gpg --dearmor -o /usr/share/keyrings/lanexadev.gpg
echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/lanexadev.gpg] https://lanexadev.github.io/apt stable main" | sudo tee /etc/apt/sources.list.d/lanexadev.list

# Installation
sudo apt update
sudo apt install muxtop
```

### Binaire pré-compilé (Linux / macOS)

```sh
curl -sSfL https://raw.githubusercontent.com/lanexadev/muxtop/main/scripts/install.sh | sh
```

### Depuis les sources

```sh
git clone https://github.com/lanexadev/muxtop.git
cd muxtop
cargo build --release
# Binaire disponible dans target/release/muxtop
```

> MSRV : Rust **1.88**

---

## Fonctionnalités

| Fonctionnalité | Détail |
|---|---|
| **Onglets** | General, Processes, Network et Containers — `Alt+1` / `Alt+2` / `Alt+3` / `Alt+4` |
| **Onglet Réseau** | Tableau d'interfaces avec RX/s, TX/s, totaux, erreurs + sparklines en temps réel |
| **Onglet Conteneurs** | Docker/Podman via [bollard](https://github.com/fussybeaver/bollard) — table CPU/mémoire/réseau/IO, sparklines CPU+RX, actions `F9` stop / `F10` kill / `F11` restart, détection socket automatique |
| **Palette de commandes** | `Ctrl+P` — `kill firefox`, `sort memory`, `stop nginx`, `restart postgres`, etc. |
| **Raccourcis htop** | `F3` recherche, `F4` filtre, `F5` arbre, `F6` tri, `F9` kill, `F10` quitter |
| **Recherche fuzzy** | Propulsé par [nucleo](https://github.com/helix-editor/nucleo) (issu de l'éditeur Helix) |
| **Vue arborescente** | `F5` bascule l'affichage hiérarchique parent/enfant |
| **Renice** | `+` / `-` pour ajuster la priorité d'un processus |
| **Monitoring distant** | `--remote host:port` + `--token` pour surveiller un serveur distant via TLS chiffré |
| **TLS natif** | Chiffrement rustls (TLS 1.2/1.3), génération auto de certificats self-signed (`--tls-generate`), auth par token obligatoire |
| **Collecte asynchrone** | Basé sur tokio — le UI n'est jamais bloqué, même à 3000+ processus |
| **Thème Tokyo Night** | TrueColor natif, repli automatique sur les terminaux ANSI/16 couleurs |
| **Binaire statique** | Un seul binaire musl, aucune dépendance système |
| **Zéro télémétrie** | Aucun appel réseau côté client, jamais (voir [Vie privée](#vie-privée--télémétrie)) |

---

## Privilèges

L'accès à la socket Docker (`/var/run/docker.sock`) est **équivalent à un accès root** sur la machine hôte : tout utilisateur du groupe `docker` peut lancer un conteneur privilégié et s'évader. Pour exécuter muxtop avec un budget de privilèges minimal, utilisez **Podman en mode rootless** — la socket utilisateur (`$XDG_RUNTIME_DIR/podman/podman.sock`) est isolée par utilisateur et muxtop la détecte automatiquement. Évitez de lancer `muxtop-server` en root sur un hôte exposé : préférez un compte de service avec uniquement la socket Podman rootless montée en lecture/écriture.

---

## Utilisation

```sh
muxtop                              # lancement normal (autodétecte Docker/Podman)
muxtop --refresh 2                  # rafraîchissement toutes les 2 secondes
muxtop --filter firefox             # démarre avec un filtre de processus
muxtop --sort mem                   # tri par mémoire au démarrage
muxtop --tree                       # démarre en vue arborescente
muxtop --about                      # version, licence, déclaration de confidentialité

# Onglet Conteneurs — par défaut muxtop cherche $DOCKER_HOST, /var/run/docker.sock,
# puis les sockets Podman. Passez un chemin pour forcer, ou désactivez complètement :
muxtop --docker-socket /var/run/docker.sock   # override du socket
muxtop --no-containers                        # désactive la collecte conteneurs

# Démarrer le serveur (TLS + auth obligatoire)
muxtop-server --token "mon-secret-16chars" --tls-generate
muxtop-server --token "mon-secret-16chars" --tls-cert cert.pem --tls-key key.pem
muxtop-server --token "mon-secret-16chars" --tls-generate --bind 0.0.0.0:4242 --max-clients 10

# Monitoring distant (TLS)
muxtop --remote host:port --token "mon-secret-16chars" --tls-skip-verify  # dev
muxtop --remote host:port --token "mon-secret-16chars" --tls-ca cert.pem  # production
MUXTOP_TOKEN="mon-secret-16chars" muxtop --remote host:port --tls-ca cert.pem
```

### Raccourcis clavier

| Touche | Action |
|--------|--------|
| `Ctrl+P` | Palette de commandes |
| `Alt+1` / `Alt+2` / `Alt+3` / `Alt+4` | Changer d'onglet (General / Processes / Network / Containers) |
| `F1` | Aide |
| `F3` / `/` | Recherche |
| `F4` | Filtre de processus |
| `F5` | Vue arborescente |
| `F6` | Menu de tri |
| `F9` | Tuer le processus (onglet Processes) · Stop conteneur (onglet Containers) |
| `F10` | Force kill (SIGKILL) — processus ou conteneur selon l'onglet actif |
| `F11` | Redémarrer le conteneur (onglet Containers) |
| `q` | Quitter |
| `j` / `k` | Navigation (style vim) |
| `+` / `-` | Renice (priorité) — onglet Processes uniquement |

---

## Benchmarks

Testé sur macOS avec 500+ processus (benchmark Thomas) :

| Métrique | Cible | muxtop |
|----------|-------|--------|
| Démarrage (`--about`) | < 100 ms | ~12 ms |
| Taille du binaire | < 10 MB | **5.3 MiB** (LTO + strip) |
| FPS (TUI) | > 30 | ~60 (event-driven, idle ≈ 0 redraw) |
| RSS pic (30 s) | < 15 MiB | **11.3 MiB** (htop ~15, btop ~40) |

Lancez le benchmark vous-même :

```sh
just bench-thomas
# ou
./scripts/bench-thomas.sh
```

---

## Architecture

```
muxtop/
├── src/                         # Point d'entrée (CLI clap + bootstrap tokio)
└── crates/
    ├── muxtop-core/             # Collecte système, modèles de données, actions
    │   ├── src/collector.rs     # Boucle async sysinfo (1 Hz) + boucle conteneurs (0.5 Hz)
    │   ├── src/process.rs       # Tri, filtrage, arbre de processus
    │   ├── src/system.rs        # Snapshots CPU / mémoire / charge
    │   ├── src/network.rs       # Interfaces réseau + historique
    │   ├── src/containers.rs    # Modèle conteneurs (ContainerSnapshot, états, engine)
    │   ├── src/container_engine.rs # Trait async + détection socket Docker/Podman
    │   └── src/docker_engine.rs # Implémentation concrète via bollard
    ├── muxtop-tui/              # Interface ratatui
    │   ├── src/app.rs           # Machine à états, gestion des événements
    │   └── src/ui/              # Onglets General, Processes, Network, Containers, palette, thème
    ├── muxtop-proto/            # Protocole filaire et sérialisation binaire
    └── muxtop-server/           # Daemon TCP pour le monitoring à distance
```

---

## Développement

```sh
just check    # fmt + clippy + tests
just bench    # micro-benchmarks criterion
just dev      # vérification continue avec bacon
```

---

## Feuille de route

| Version | Objectif |
|---------|----------|
| **v0.1** ✓ | Remplacement htop — onglets, palette de commandes, vue arborescente |
| **v0.2** ✓ | Onglet Réseau (remplace iftop) + architecture client-serveur (`muxtop-server`, `--remote`) |
| **v0.3** ✓ | Onglet Conteneurs Docker / Podman (via [bollard](https://github.com/fussybeaver/bollard)) + actions Stop/Kill/Restart |
| v0.3.1 | Support Kubernetes (via [kube-rs](https://github.com/kube-rs/kube)) |
| v0.4 | Monitoring GPU (NVIDIA / AMD / Apple Silicon) + `docker exec` interactif (PTY) |
| v1.0 | Système de plugins WASM + thèmes + fichier de configuration |

---

## Vie privée & télémétrie

muxtop ne collecte **AUCUNE** télémétrie, **AUCUNE** statistique et ne contacte **PERSONNE**. Jamais.

Il ne fait aucun appel réseau. Il est conçu pour les serveurs de production air-gappés.
Si vous observez une activité réseau sortante depuis muxtop, c'est un bug — veuillez le [signaler](https://github.com/lanexadev/muxtop/issues).

---

## Contribuer

Les contributions sont les bienvenues ! Consultez [CONTRIBUTING.md](CONTRIBUTING.md) pour les prérequis, les conventions de code, le workflow de branches et les instructions pour soumettre une PR.

---

## Licence

Disponible sous l'un ou l'autre des licences suivantes, à votre choix :

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))
