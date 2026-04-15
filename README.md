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

### Via .deb (Debian / Ubuntu)

Téléchargez le `.deb` correspondant à votre architecture depuis la [dernière release](https://github.com/lanexadev/muxtop/releases/latest) :

```sh
# x86_64
wget https://github.com/lanexadev/muxtop/releases/latest/download/muxtop-x86_64-unknown-linux-musl.deb
sudo dpkg -i muxtop-x86_64-unknown-linux-musl.deb

# aarch64
wget https://github.com/lanexadev/muxtop/releases/latest/download/muxtop-aarch64-unknown-linux-musl.deb
sudo dpkg -i muxtop-aarch64-unknown-linux-musl.deb
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
| **Onglets** | General (CPU, mémoire, charge) et Processes — `Alt+1` / `Alt+2` |
| **Palette de commandes** | `Ctrl+P` — `kill firefox`, `sort memory`, etc. |
| **Raccourcis htop** | `F3` recherche, `F4` filtre, `F5` arbre, `F6` tri, `F9` kill, `F10` quitter |
| **Recherche fuzzy** | Propulsé par [nucleo](https://github.com/helix-editor/nucleo) (issu de l'éditeur Helix) |
| **Vue arborescente** | `F5` bascule l'affichage hiérarchique parent/enfant |
| **Renice** | `+` / `-` pour ajuster la priorité d'un processus |
| **Collecte asynchrone** | Basé sur tokio — le UI n'est jamais bloqué, même à 3000+ processus |
| **Thème Tokyo Night** | TrueColor natif, repli automatique sur les terminaux ANSI/16 couleurs |
| **Binaire statique** | Un seul binaire musl, aucune dépendance système |
| **Zéro télémétrie** | Aucun appel réseau, jamais (voir [Vie privée](#vie-privée--télémétrie)) |

---

## Utilisation

```sh
muxtop                          # lancement normal
muxtop --refresh 2              # rafraîchissement toutes les 2 secondes
muxtop --filter firefox         # démarre avec un filtre de processus
muxtop --sort mem               # tri par mémoire au démarrage
muxtop --tree                   # démarre en vue arborescente
muxtop --about                  # version, licence, déclaration de confidentialité
```

### Raccourcis clavier

| Touche | Action |
|--------|--------|
| `Ctrl+P` | Palette de commandes |
| `Alt+1` / `Alt+2` | Changer d'onglet |
| `F1` | Aide |
| `F3` / `/` | Recherche |
| `F4` | Filtre de processus |
| `F5` | Vue arborescente |
| `F6` | Menu de tri |
| `F9` | Tuer le processus |
| `F10` / `q` | Quitter |
| `j` / `k` | Navigation (style vim) |
| `+` / `-` | Renice (priorité) |

---

## Benchmarks

Testé sur macOS avec 500+ processus (benchmark Thomas) :

| Métrique | Cible | muxtop |
|----------|-------|--------|
| Démarrage (`--about`) | < 100 ms | ~5 ms |
| Taille du binaire | < 10 MB | ~4 MB |
| FPS (TUI) | > 30 | ~60 |
| RAM | < 10 MB | < 8 MB |

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
├── src/                    # Point d'entrée (CLI clap + bootstrap tokio)
└── crates/
    ├── muxtop-core/        # Collecte système, modèles de données, actions
    │   ├── src/collector.rs  # Boucle async sysinfo
    │   ├── src/process.rs    # Tri, filtrage, arbre de processus
    │   └── src/system.rs     # Snapshots CPU / mémoire / charge
    └── muxtop-tui/         # Interface ratatui
        ├── src/app.rs        # Machine à états, gestion des événements
        └── src/ui/           # Onglets General, Processes, palette, thème
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
| **v0.1** | Remplacement htop — onglets, palette de commandes, vue arborescente |
| v0.2 | Onglet Réseau (remplace iftop) + architecture client-serveur |
| v0.3 | Onglet Conteneurs (Docker / Podman / K8s) + monitoring GPU |
| v1.0 | Système de plugins WASM + thèmes + fichier de configuration |

---

## Vie privée & télémétrie

muxtop ne collecte **AUCUNE** télémétrie, **AUCUNE** statistique et ne contacte **PERSONNE**. Jamais.

Il ne fait aucun appel réseau. Il est conçu pour les serveurs de production air-gappés.
Si vous observez une activité réseau sortante depuis muxtop, c'est un bug — veuillez le [signaler](https://github.com/lanexadev/muxtop/issues).

---

## Contribuer

Les contributions sont les bienvenues. Merci d'ouvrir d'abord une issue pour discuter des changements envisagés.

---

## Licence

Disponible sous l'un ou l'autre des licences suivantes, à votre choix :

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))
