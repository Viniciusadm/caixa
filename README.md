# Caixa

Caixa uploads files to Google Drive and prints a download URL. It is not a Drive client: the `cx` binary uses the `drive.file` scope, so it only sees the files it uploaded itself, which go to a **caixa** folder in your Drive. This repository holds that command line tool; its messages are in Portuguese.

## Requirements

- Rust **1.88+** (edition 2024, uses let chains)
- A Google Cloud OAuth client of type *Desktop app* with the Google Drive API enabled

## Install

Install the `cx` binary into `~/.cargo/bin`, store the OAuth client and sign in (the login opens the browser and uses a local redirect with PKCE):

```sh
cargo install --path .
cx config
cx login
```

The config is `~/.config/caixa/config.toml`, written with mode 600, and holds the client credentials, the refresh token and the folder id. `CX_CLIENT_ID` and `CX_CLIENT_SECRET` override the file.

### Google credentials

This is a one-time setup in [console.cloud.google.com](https://console.cloud.google.com):

1. Create a project (for example `caixa`).
2. *APIs & Services, Library*: enable the **Google Drive API**.
3. *OAuth consent screen*: user type **Internal**, app name `caixa`, your email. No scopes need to be added.
4. *Credentials, Create credentials, OAuth client ID*: type **Desktop app**.
5. Paste the client ID and secret into `cx config`.

An **Internal** app (Workspace accounts only) skips Google's verification and its login does not expire. If the Workspace admin blocks new projects or OAuth apps, ask them.

## Usage

Upload files; each line of output is a download URL:

```sh
cx up report.pdf photos.zip
cx up video.mp4 | xclip -sel c
```

By default each file is shared as "anyone with the link: viewer". `-p` skips sharing, so the link only works for people with access. If the Workspace policy forbids link sharing, the upload still succeeds with a warning and a restricted link.

List, fetch and remove what was uploaded:

```sh
cx ls
cx url <id|link>
cx get <id|link>
cx get <id|link> -o ~/Downloads
cx rm <id|link>
```

`ls` takes `--json` for scripts. `get` downloads with the original name into the current folder, `rm` moves the file to the Drive trash. `get`, `url` and `rm` accept the id, the download URL (`.../uc?export=download&id=...`) or the view link (`.../file/d/<id>/view`).

Files above about 100 MB show Google's "can't scan for viruses" page before a browser download; `cx get` does not go through it.

## Tests

```sh
cargo test
```

`cargo test` covers parsing file ids out of Drive links, building download URLs and reading and formatting file sizes from the API.
