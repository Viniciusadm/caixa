use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::blocking::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use url::Url;

use crate::config::Config;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct TokenError {
    error: String,
    error_description: Option<String>,
}

pub fn login(http: &Client, config: &Config) -> Result<String> {
    let (client_id, client_secret) = config.credentials()?;
    let listener = TcpListener::bind("127.0.0.1:0").context("não consegui abrir uma porta local")?;
    let redirect_uri = format!("http://127.0.0.1:{}", listener.local_addr()?.port());

    let verifier = URL_SAFE_NO_PAD.encode(random_bytes(32)?);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = URL_SAFE_NO_PAD.encode(random_bytes(16)?);

    let auth_url = Url::parse_with_params(
        AUTH_URL,
        &[
            ("client_id", client_id),
            ("redirect_uri", redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", SCOPE),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.as_str()),
        ],
    )?;

    eprintln!("Abrindo o navegador para login. Se não abrir, acesse:\n{auth_url}");
    crate::open_url(auth_url.as_str());

    let code = wait_for_code(&listener, &state)?;

    let response = http
        .post(TOKEN_URL)
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri.as_str()),
        ])
        .send()?;
    let token = parse_token(response)?;
    token
        .refresh_token
        .context("o Google não devolveu refresh_token; tente `cx login` de novo")
}

pub fn access_token(http: &Client, config: &Config) -> Result<String> {
    let (client_id, client_secret) = config.credentials()?;
    let refresh_token = config.refresh_token()?;
    let response = http
        .post(TOKEN_URL)
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()?;
    Ok(parse_token(response)?.access_token)
}

fn parse_token(response: reqwest::blocking::Response) -> Result<TokenResponse> {
    if response.status().is_success() {
        return Ok(response.json()?);
    }
    let status = response.status();
    let body = response.text().unwrap_or_default();
    match serde_json::from_str::<TokenError>(&body) {
        Ok(error) if error.error == "invalid_grant" => bail!("sessão expirada ou revogada (rode `cx login`)"),
        Ok(error) => bail!(
            "falha na autenticação: {} {}",
            error.error,
            error.error_description.unwrap_or_default()
        ),
        Err(_) => bail!("falha na autenticação ({status}): {body}"),
    }
}

fn wait_for_code(listener: &TcpListener, expected_state: &str) -> Result<String> {
    loop {
        let (mut stream, _) = listener.accept()?;
        let mut request_line = String::new();
        BufReader::new(&stream).read_line(&mut request_line)?;
        let Some(target) = request_line.split_whitespace().nth(1) else {
            continue;
        };
        let callback = Url::parse(&format!("http://127.0.0.1{target}"))?;
        let param = |name: &str| {
            callback
                .query_pairs()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
        };
        if param("code").is_none() && param("error").is_none() {
            respond(&mut stream, "")?;
            continue;
        }

        if let Some(error) = param("error") {
            respond(&mut stream, "Login cancelado. Pode fechar esta aba.")?;
            bail!("login recusado: {error}");
        }
        if param("state").as_deref() != Some(expected_state) {
            respond(&mut stream, "Resposta inválida. Pode fechar esta aba.")?;
            bail!("state do OAuth não confere");
        }
        respond(&mut stream, "Login feito! Pode fechar esta aba e voltar ao terminal.")?;
        return Ok(param("code").unwrap_or_default());
    }
}

fn respond(stream: &mut impl Write, message: &str) -> Result<()> {
    let body = format!("<!doctype html><meta charset=\"utf-8\"><title>caixa</title><p>{message}</p>");
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    Ok(())
}

fn random_bytes(count: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![0; count];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes)
}
