use anyhow::{Result, bail};
use url::Url;

pub fn download_url(id: &str) -> String {
    format!("https://drive.google.com/uc?export=download&id={id}")
}

pub fn parse_file_ref(input: &str) -> Result<String> {
    let input = input.trim();
    if is_file_id(input) {
        return Ok(input.to_string());
    }
    let Ok(url) = Url::parse(input) else {
        bail!("não reconheci \"{input}\" como id ou link do Drive");
    };
    if let Some((_, id)) = url.query_pairs().find(|(key, _)| key == "id")
        && is_file_id(&id)
    {
        return Ok(id.into_owned());
    }
    let segments: Vec<&str> = url.path_segments().map(Iterator::collect).unwrap_or_default();
    if let Some(position) = segments.iter().position(|segment| *segment == "d")
        && let Some(id) = segments.get(position + 1).filter(|id| is_file_id(id))
    {
        return Ok(id.to_string());
    }
    bail!("não achei o id do arquivo em {input}")
}

fn is_file_id(value: &str) -> bool {
    value.len() >= 10
        && value
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || char == '-' || char == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "1AbC_dEf-GhIjKlMnOpQ";

    #[test]
    fn accepts_plain_id() {
        assert_eq!(parse_file_ref(ID).unwrap(), ID);
    }

    #[test]
    fn extracts_id_from_urls() {
        for url in [
            format!("https://drive.google.com/file/d/{ID}/view?usp=sharing"),
            format!("https://drive.google.com/uc?export=download&id={ID}"),
            format!("https://drive.google.com/open?id={ID}"),
            format!("https://docs.google.com/document/d/{ID}/edit"),
        ] {
            assert_eq!(parse_file_ref(&url).unwrap(), ID, "{url}");
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_file_ref("arquivo.pdf").is_err());
        assert!(parse_file_ref("https://example.com/nada").is_err());
    }

    #[test]
    fn builds_download_url() {
        assert_eq!(
            download_url(ID),
            format!("https://drive.google.com/uc?export=download&id={ID}")
        );
    }
}
