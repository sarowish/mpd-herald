use crate::mpd::SongInfo;
use anyhow::Result;
use mpd_client::tag::Tag;
use reqwest::Client;
use serde::Deserialize;
use std::{collections::HashMap, fmt::Display, sync::LazyLock};
use tokio::sync::Mutex;

const MUSICBRAINZ_RELEASE_GROUP_SEARCH: &str = "https://musicbrainz.org/ws/2/release-group/";
const MUSICBRAINZ_USER_AGENT: &str = concat!(
    env!("CARGO_PKG_NAME"),
    "/",
    env!("MPD_HERALD_BUILD_VERSION"),
    " (https://github.com/sarowish/mpd-herald)"
);

enum ReleaseType {
    Release,
    ReleaseGroup,
}

impl Display for ReleaseType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Release => String::from("release"),
                Self::ReleaseGroup => String::from("release-group"),
            }
        )
    }
}

#[derive(Default)]
struct AlbumArtClient {
    client: Client,
    image_urls: HashMap<String, String>,
    release_group_ids: HashMap<AlbumKey, String>,
}

#[derive(Eq, Hash, PartialEq)]
struct AlbumKey {
    artist: String,
    album: String,
}

impl AlbumKey {
    fn from_song(song: &SongInfo) -> Result<Self> {
        let artist = song
            .single_tag_value(&Tag::AlbumArtist)
            .or_else(|| song.single_tag_value(&Tag::Artist))
            .ok_or_else(|| anyhow::anyhow!("No artist tag"))?;
        let album = song
            .single_tag_value(&Tag::Album)
            .ok_or_else(|| anyhow::anyhow!("No album tag"))?;

        Ok(Self {
            artist: artist.to_owned(),
            album: album.to_owned(),
        })
    }
}

#[derive(Deserialize)]
struct ReleaseGroupSearch {
    #[serde(rename = "release-groups")]
    release_groups: Vec<ReleaseGroup>,
}

#[derive(Deserialize)]
struct ReleaseGroup {
    id: String,
}

impl AlbumArtClient {
    async fn lookup_release_group_id(&mut self, song: &SongInfo) -> Result<String> {
        let album = AlbumKey::from_song(song)?;

        if let Some(id) = self.release_group_ids.get(&album) {
            return Ok(id.clone());
        }

        let query = release_group_query(&album.artist, &album.album);
        let search = self
            .client
            .get(MUSICBRAINZ_RELEASE_GROUP_SEARCH)
            .header(reqwest::header::USER_AGENT, MUSICBRAINZ_USER_AGENT)
            .query(&[("query", query.as_str()), ("fmt", "json"), ("limit", "1")])
            .send()
            .await?
            .error_for_status()?
            .json::<ReleaseGroupSearch>()
            .await?;
        let id = search
            .release_groups
            .into_iter()
            .next()
            .map(|release_group| release_group.id)
            .ok_or_else(|| anyhow::anyhow!("No musicbrainz release group found"))?;

        self.release_group_ids.insert(album, id.clone());

        Ok(id)
    }
}

#[rustfmt::skip::macros(matches)]
fn escape_lucene(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());

    for character in value.chars() {
        if matches!(
            character,
            '+' | '-' | '&' | '|' | '!' |
            '(' | ')' | '{' | '}' | '[' |
            ']' | '^' | '"' | '~' | '*' |
            '?' | ':' | '/' | '\\'
        ) {
            escaped.push('\\');
        }

        escaped.push(character);
    }

    escaped
}

fn release_group_query(artist: &str, album: &str) -> String {
    format!(
        "artist:\"{}\" AND release:\"{}\"",
        escape_lucene(artist),
        escape_lucene(album)
    )
}

pub async fn get(song: &SongInfo) -> Result<String> {
    static CLIENT: LazyLock<Mutex<AlbumArtClient>> =
        LazyLock::new(|| Mutex::new(AlbumArtClient::default()));

    let mut guard = CLIENT.lock().await;

    let release_group_id_tag = Tag::Other("MUSICBRAINZ_RELEASEGROUPID".into());
    let (rel_type, id) = if let Some(release) = song
        .single_tag_value(&Tag::MusicBrainzReleaseId)
        .map(|id| (ReleaseType::Release, id.to_owned()))
        .or_else(|| {
            song.single_tag_value(&release_group_id_tag)
                .map(|id| (ReleaseType::ReleaseGroup, id.to_owned()))
        }) {
        release
    } else {
        (
            ReleaseType::ReleaseGroup,
            guard.lookup_release_group_id(song).await?,
        )
    };

    if let Some(url) = guard.image_urls.get(&id) {
        return Ok(url.to_owned());
    }

    let url = format!("https://coverartarchive.org/{rel_type}/{id}/front-250");
    let Ok(resp) = guard.client.get(&url).send().await else {
        return Ok(url);
    };

    if let 200 | 307 = resp.status().as_u16() {
        let url = resp.url().to_string();
        guard.image_urls.insert(id, url.clone());
        return Ok(url);
    } else if matches!(rel_type, ReleaseType::ReleaseGroup) {
        return Err(anyhow::anyhow!("No image"));
    }

    let Some(url) = song
        .single_tag_value(&release_group_id_tag)
        .map(|id| format!("https://coverartarchive.org/release-group/{id}/front-250"))
    else {
        return Err(anyhow::anyhow!("No musicbrainz release group id"));
    };

    Ok(match guard.client.get(&url).send().await {
        Ok(resp) if matches!(resp.status().as_u16(), 200 | 307) => resp.url().to_string(),
        _ => url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_release_group_query_from_escaped_artist_and_album() {
        assert_eq!(
            release_group_query("AC/DC", "Hits: 80's + 90's"),
            r#"artist:"AC\/DC" AND release:"Hits\: 80's \+ 90's""#
        );
    }
}
