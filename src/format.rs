use crate::{mpd::SongInfo, utils::duration_as_hhmmss};
use mpd_client::tag::Tag;
use regex_lite::Regex;
use serde::{Deserialize, de};
use std::{borrow::Cow, sync::LazyLock};

enum TokenKind {
    Metadata(Tag),
    Time,
    Elapsed,
    File,
    Other(String),
}

impl TokenKind {
    fn get_value<'a>(&'a self, song: &'a SongInfo) -> Cow<'a, str> {
        match self {
            Self::Metadata(tag) => Cow::Borrowed(song.single_tag_value(tag).unwrap_or_default()),
            Self::Time => Cow::Owned(duration_as_hhmmss(song.duration)),
            Self::Elapsed => Cow::Owned(duration_as_hhmmss(song.elapsed)),
            Self::File => Cow::Borrowed(&song.url),
            Self::Other(s) => Cow::Borrowed(s),
        }
    }
}

impl From<&str> for TokenKind {
    fn from(token: &str) -> Self {
        match token.trim_matches('%') {
            "name" => Self::Metadata(Tag::Name),
            "artist" => Self::Metadata(Tag::Artist),
            "album" => Self::Metadata(Tag::Album),
            "albumartist" => Self::Metadata(Tag::AlbumArtist),
            "composer" => Self::Metadata(Tag::Composer),
            "date" => Self::Metadata(Tag::Date),
            "originaldate" => Self::Metadata(Tag::OriginalDate),
            "disc" => Self::Metadata(Tag::Disc),
            "genre" => Self::Metadata(Tag::Genre),
            "performer" => Self::Metadata(Tag::Performer),
            "title" => Self::Metadata(Tag::Title),
            "track" => Self::Metadata(Tag::Track),
            "time" => Self::Time,
            "elapsed" => Self::Elapsed,
            "file" => Self::File,
            s => Self::Other(s.to_owned()),
        }
    }
}

enum Part {
    Literal(String),
    Token(TokenKind),
}

#[derive(Default)]
pub struct CompiledFormat {
    parts: Vec<Part>,
}

impl CompiledFormat {
    pub fn new(format: &str) -> Self {
        static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"%\w+%").unwrap());

        let mut parts = Vec::new();
        let mut cursor = 0;

        for token in RE.find_iter(format) {
            if cursor < token.start() {
                parts.push(Part::Literal(format[cursor..token.start()].to_owned()));
            }

            parts.push(Part::Token(token.as_str().into()));
            cursor = token.end();
        }

        if cursor < format.len() {
            parts.push(Part::Literal(format[cursor..].to_owned()));
        }

        Self { parts }
    }

    pub fn render(&self, song: &SongInfo) -> String {
        let mut replaced = String::new();

        for part in &self.parts {
            match part {
                Part::Literal(s) => replaced.push_str(s),
                Part::Token(kind) => replaced.push_str(&kind.get_value(song)),
            };
        }

        replaced
    }
}

impl<'de> Deserialize<'de> for CompiledFormat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let format: String = de::Deserialize::deserialize(deserializer)?;

        Ok(CompiledFormat::new(&format))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpd_client::responses::PlayState;
    use std::{collections::HashMap, time::Duration};

    fn song_with_tags(tags: impl IntoIterator<Item = (Tag, &'static str)>) -> SongInfo {
        SongInfo {
            url: "music/track.flac".to_owned(),
            state: PlayState::Playing,
            elapsed: Some(Duration::from_secs(67)),
            duration: Some(Duration::from_secs(6767)),
            tags: tags
                .into_iter()
                .map(|(tag, value)| (tag, vec![value.to_owned()]))
                .collect::<HashMap<_, _>>(),
            fired_at: 0,
        }
    }

    fn empty_song() -> SongInfo {
        song_with_tags([])
    }

    #[test]
    fn renders_adjacent_and_repeated_tokens() {
        let format = CompiledFormat::new("%artist%%title%/%artist%");
        let song = song_with_tags([(Tag::Artist, "Artist"), (Tag::Title, "Title")]);

        assert_eq!(format.render(&song), "ArtistTitle/Artist");
    }

    #[test]
    fn renders_missing_values_as_empty_strings() {
        let format = CompiledFormat::new("[%artist%] %title% (%album%)");

        assert_eq!(format.render(&empty_song()), "[]  ()");
    }

    #[test]
    fn renders_unknown_tokens_without_percent_signs() {
        let format = CompiledFormat::new("before %unknown_token% after");

        assert_eq!(format.render(&empty_song()), "before unknown_token after");
    }

    #[test]
    fn empty_and_default_formats_render_empty_strings() {
        assert_eq!(CompiledFormat::new("").render(&empty_song()), "");
        assert_eq!(CompiledFormat::default().render(&empty_song()), "");
    }
}
