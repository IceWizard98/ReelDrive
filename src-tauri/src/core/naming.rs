//! Filename and folder-name parsing. Pure string work, no regex crate: the
//! patterns are few and fully covered by tests.

use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpisodeRef {
    pub season: Option<u32>,
    pub episode: u32,
}

const YEAR_MIN: u16 = 1888;
const YEAR_MAX: u16 = 2099;

const SEASON_PREFIXES: &[&str] = &["stagione", "season", "s"];
const SPECIALS_NAMES: &[&str] = &["specials", "special", "extras", "extra", "bonus"];
const EPISODE_WORDS: &[&str] = &["episodio", "episode", "ep"];

/// Replace the separators release folders use with spaces and collapse runs.
fn normalize(name: &str) -> String {
    let spaced: String = name
        .chars()
        .map(|c| if c == '.' || c == '_' { ' ' } else { c })
        .collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn as_year(token: &str) -> Option<u16> {
    let digits = token.trim_matches(|c| c == '(' || c == ')' || c == '[' || c == ']');
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits
        .parse::<u16>()
        .ok()
        .filter(|y| (YEAR_MIN..=YEAR_MAX).contains(y))
}

/// Split a content folder name into a display title and an optional year.
pub fn clean_title(name: &str) -> (String, Option<u16>) {
    let normalized = normalize(name);
    let tokens: Vec<&str> = normalized.split(' ').filter(|t| !t.is_empty()).collect();

    // A bracketed year is unambiguous. A bare one only counts when something
    // follows it, otherwise it is part of the title ("Blade Runner 2049").
    let bracketed = tokens
        .iter()
        .position(|t| t.starts_with('(') || t.starts_with('['))
        .filter(|&i| as_year(tokens[i]).is_some());
    let bare = tokens
        .iter()
        .position(|t| as_year(t).is_some() && !t.starts_with('(') && !t.starts_with('['))
        .filter(|&i| i + 1 < tokens.len());

    let year_at = bracketed.or(bare);
    match year_at {
        Some(i) => {
            let title = tokens[..i].join(" ");
            if title.is_empty() {
                (normalized, None)
            } else {
                (title, as_year(tokens[i]))
            }
        }
        None => (normalized, None),
    }
}

/// Season number for a folder name, `None` if it does not look like a season.
/// Specials and extras map to season 0.
pub fn parse_season_dir(name: &str) -> Option<u32> {
    let lowered = name.trim().to_ascii_lowercase();
    if lowered.is_empty() {
        return None;
    }
    if SPECIALS_NAMES.contains(&lowered.as_str()) {
        return Some(0);
    }
    for prefix in SEASON_PREFIXES {
        let Some(rest) = lowered.strip_prefix(prefix) else {
            continue;
        };
        let rest = rest.trim_start_matches([' ', '_', '.', '-']);
        // Saturating rather than `parse().ok()`: a season number too large for
        // a u32 would otherwise stop the folder being a season at all, and its
        // episodes would vanish from the library.
        let Some((number, end)) = read_number(rest.as_bytes(), 0) else {
            continue;
        };
        // Whatever follows the number is the user's own label — "Season 1 -
        // inizio", "Stagione 2 — La vendetta". Requiring the name to *end* at
        // the number meant a folder named that way was not a season at all: its
        // episodes fell through to the unnamed-folder fallback, which numbers
        // by position, so "Season 3" on its own could come out as season 1.
        //
        // Separated from the number, though, not glued to it: `S01E02` is a
        // file's marker and never a season folder, and `Season 1x02` is not one
        // either. One alphanumeric character touching the digits is the whole
        // difference.
        let glued = rest
            .as_bytes()
            .get(end)
            .is_some_and(u8::is_ascii_alphanumeric);
        if !glued {
            return Some(number);
        }
    }
    None
}

/// The name to show for a season folder, when the folder says more than its
/// own number.
///
/// `Season 2` and `S02` and `stagione 2` all mean the same thing and are better
/// shown as one canonical label, which the window builds from the number.
/// `Season 1 - inizio` is not that: the part after the number is the user's,
/// nobody else knows it, and dropping it makes two folders they named
/// differently look identical in the list. Whatever they typed comes back,
/// tidied of the separators release folders use and nothing else.
pub fn season_dir_title(name: &str) -> Option<String> {
    let tidy = normalize(name);
    if tidy.is_empty() {
        return None;
    }
    // Season zero is never shown as a number, so its folder name is all there
    // is to call it: "Specials", "Extras", "Bonus". A folder that is not a
    // recognised season at all — "Prima Stagione", "Parte 1", the ones the
    // fallback turns into seasons by position — is in the same position: its
    // name is the only thing anyone knows about it.
    if parse_season_dir(name) == Some(0) {
        return Some(tidy);
    }
    // Only the marker: the canonical label says the same thing more clearly.
    let lowered = tidy.to_ascii_lowercase();
    let bare = SEASON_PREFIXES.iter().any(|prefix| {
        lowered
            .strip_prefix(prefix)
            .map(|rest| rest.trim_start_matches([' ', '_', '.', '-']))
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
    });
    if bare {
        None
    } else {
        Some(tidy)
    }
}

/// True when the byte at `i` starts a token (start of string or after a separator).
fn at_token_start(bytes: &[u8], i: usize) -> bool {
    i == 0 || !bytes[i - 1].is_ascii_alphanumeric()
}

/// Read a run of ASCII digits starting at `i`, returning the value and the
/// index just past the run. Runs longer than a `u32` saturate rather than wrap.
fn read_number(bytes: &[u8], i: usize) -> Option<(u32, usize)> {
    let mut end = i;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == i {
        return None;
    }
    let value = bytes[i..end].iter().fold(0u32, |acc, b| {
        acc.saturating_mul(10).saturating_add(u32::from(b - b'0'))
    });
    Some((value, end))
}

fn eq_ignore_case_at(bytes: &[u8], i: usize, word: &str) -> bool {
    let w = word.as_bytes();
    bytes.len() >= i + w.len()
        && bytes[i..i + w.len()]
            .iter()
            .zip(w)
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
}

/// Locate the first episode marker and return it with the index just past it.
/// Markers are ASCII, so byte scanning is safe on UTF-8 input.
fn find_marker(stem: &str) -> Option<(EpisodeRef, usize)> {
    let bytes = stem.as_bytes();

    // SxxEyy
    for i in 0..bytes.len() {
        if !bytes[i].eq_ignore_ascii_case(&b's') || !at_token_start(bytes, i) {
            continue;
        }
        let Some((season, after_season)) = read_number(bytes, i + 1) else {
            continue;
        };
        if after_season >= bytes.len() || !bytes[after_season].eq_ignore_ascii_case(&b'e') {
            continue;
        }
        if let Some((episode, end)) = read_number(bytes, after_season + 1) {
            return Some((
                EpisodeRef {
                    season: Some(season),
                    episode,
                },
                end,
            ));
        }
    }

    // 1x03
    for i in 0..bytes.len() {
        if !bytes[i].is_ascii_digit() || !at_token_start(bytes, i) {
            continue;
        }
        let Some((season, after_season)) = read_number(bytes, i) else {
            continue;
        };
        if after_season >= bytes.len() || !bytes[after_season].eq_ignore_ascii_case(&b'x') {
            continue;
        }
        if let Some((episode, end)) = read_number(bytes, after_season + 1) {
            return Some((
                EpisodeRef {
                    season: Some(season),
                    episode,
                },
                end,
            ));
        }
    }

    // "Episodio 4" / "Episode 4" / "Ep 4" — longest word first so "ep" does not
    // shadow "episodio".
    for i in 0..bytes.len() {
        if !at_token_start(bytes, i) {
            continue;
        }
        for word in EPISODE_WORDS {
            if !eq_ignore_case_at(bytes, i, word) {
                continue;
            }
            let mut j = i + word.len();
            while j < bytes.len() && matches!(bytes[j], b' ' | b'_' | b'.' | b'-') {
                j += 1;
            }
            if let Some((episode, end)) = read_number(bytes, j) {
                return Some((
                    EpisodeRef {
                        season: None,
                        episode,
                    },
                    end,
                ));
            }
        }
    }

    // Bare "E05"
    for i in 0..bytes.len() {
        if !bytes[i].eq_ignore_ascii_case(&b'e') || !at_token_start(bytes, i) {
            continue;
        }
        if let Some((episode, end)) = read_number(bytes, i + 1) {
            return Some((
                EpisodeRef {
                    season: None,
                    episode,
                },
                end,
            ));
        }
    }

    // Leading "04 - Title": at most three digits, then a separator or the end.
    if let Some((episode, end)) = read_number(bytes, 0) {
        let followed_by_separator = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if end <= 3 && followed_by_separator {
            return Some((
                EpisodeRef {
                    season: None,
                    episode,
                },
                end,
            ));
        }
    }

    None
}

/// Season/episode markers inside a file stem.
pub fn parse_episode(stem: &str) -> Option<EpisodeRef> {
    find_marker(stem).map(|(reference, _)| reference)
}

/// Human title of an episode: whatever follows the episode marker, cleaned up.
/// Empty when the filename carries no title of its own.
pub fn episode_title(stem: &str) -> String {
    match find_marker(stem) {
        Some((_, end)) => {
            let rest = stem[end..].trim_matches(|c: char| {
                c.is_whitespace() || matches!(c, '-' | '_' | '.' | '–' | '—' | '|')
            });
            normalize(rest)
        }
        None => normalize(stem),
    }
}

/// Case-insensitive comparison where digit runs compare numerically, so
/// `ep2` sorts before `ep10`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);

    while i < x.len() && j < y.len() {
        if x[i].is_ascii_digit() && y[j].is_ascii_digit() {
            let start_x = i;
            let start_y = j;
            while i < x.len() && x[i].is_ascii_digit() {
                i += 1;
            }
            while j < y.len() && y[j].is_ascii_digit() {
                j += 1;
            }
            // Compare without parsing, so arbitrarily long runs stay correct.
            let dx = trim_leading_zeros(&x[start_x..i]);
            let dy = trim_leading_zeros(&y[start_y..j]);
            match dx.len().cmp(&dy.len()).then_with(|| dx.cmp(dy)) {
                Ordering::Equal => continue,
                other => return other,
            }
        }

        match x[i].to_ascii_lowercase().cmp(&y[j].to_ascii_lowercase()) {
            Ordering::Equal => {
                i += 1;
                j += 1;
            }
            other => return other,
        }
    }

    (x.len() - i).cmp(&(y.len() - j))
}

fn trim_leading_zeros(digits: &[u8]) -> &[u8] {
    let first_significant = digits
        .iter()
        .position(|&b| b != b'0')
        .unwrap_or(digits.len());
    &digits[first_significant..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_title_keeps_plain_name() {
        assert_eq!(clean_title("Scrubs"), ("Scrubs".to_string(), None));
    }

    #[test]
    fn clean_title_extracts_parenthesized_year() {
        assert_eq!(
            clean_title("Inception (2010)"),
            ("Inception".to_string(), Some(2010))
        );
    }

    #[test]
    fn clean_title_replaces_dots_and_strips_release_tags() {
        assert_eq!(
            clean_title("The.Matrix.1999.1080p.BluRay.x264"),
            ("The Matrix".to_string(), Some(1999))
        );
    }

    #[test]
    fn clean_title_replaces_underscores() {
        assert_eq!(
            clean_title("How_I_Met_Your_Mother"),
            ("How I Met Your Mother".to_string(), None)
        );
    }

    #[test]
    fn clean_title_keeps_trailing_number_that_is_part_of_the_title() {
        // "2049" is the last token, so it belongs to the title, not to a year.
        assert_eq!(
            clean_title("Blade Runner 2049"),
            ("Blade Runner 2049".to_string(), None)
        );
    }

    #[test]
    fn clean_title_prefers_parenthesized_year_over_title_number() {
        assert_eq!(
            clean_title("Blade Runner 2049 (2017)"),
            ("Blade Runner 2049".to_string(), Some(2017))
        );
    }

    #[test]
    fn clean_title_collapses_repeated_separators() {
        assert_eq!(
            clean_title("  Dune -- Part  Two  "),
            ("Dune -- Part Two".to_string(), None)
        );
    }

    #[test]
    fn clean_title_handles_unicode() {
        assert_eq!(
            clean_title("Amélie (2001)"),
            ("Amélie".to_string(), Some(2001))
        );
    }

    #[test]
    fn clean_title_never_returns_empty_title() {
        assert_eq!(clean_title("(2010)"), ("(2010)".to_string(), None));
    }

    #[test]
    fn parse_season_dir_accepts_known_spellings() {
        for (name, expected) in [
            ("Season 1", 1),
            ("season 01", 1),
            ("SEASON  2", 2),
            ("S02", 2),
            ("s3", 3),
            ("Stagione 3", 3),
            ("Season_4", 4),
            ("Season.5", 5),
            ("Season-6", 6),
        ] {
            assert_eq!(parse_season_dir(name), Some(expected), "name: {name}");
        }
    }

    #[test]
    fn parse_season_dir_saturates_absurd_numbers_instead_of_giving_up() {
        // Losing the match would drop every episode in the folder.
        assert_eq!(
            parse_season_dir("Season 99999999999999999999"),
            Some(u32::MAX)
        );
    }

    #[test]
    fn parse_season_dir_accepts_a_label_after_the_number() {
        // What people actually type. Requiring the name to end at the number
        // meant every one of these was not a season: the episodes inside fell
        // through to the fallback that numbers folders by position, so a lone
        // "Season 3 - Finale" came out as season 1.
        for (name, expected) in [
            ("Season 1 - inizio", 1),
            ("Season 1 — Inizio", 1),
            ("Season 1- inizio", 1),
            ("Season 1: Beginnings", 1),
            ("Stagione 2 La vendetta", 2),
            ("S03 - Finale", 3),
            ("season 04 (2004)", 4),
            ("Season 5.The End", 5),
            ("s6_extra", 6),
        ] {
            assert_eq!(parse_season_dir(name), Some(expected), "name: {name}");
        }
    }

    #[test]
    fn parse_season_dir_refuses_a_marker_glued_to_the_number() {
        // The one thing the label rule must not swallow. `S01E02` is a file's
        // episode marker; read as a season folder it would make every episode
        // inside a series folder into a season of its own. One alphanumeric
        // character touching the digits is the whole difference.
        for name in [
            "S01E02",
            "s1e1",
            "Season 1x02",
            "S02E03E04",
            "s1e2 - Pilot",
            "Season 1080p",
        ] {
            assert_eq!(parse_season_dir(name), None, "name: {name}");
        }
    }

    #[test]
    fn parse_season_dir_still_refuses_titles_that_merely_start_with_s() {
        // The bare "s" prefix is the greedy one, and a library is full of
        // titles beginning with it.
        for name in [
            "Scrubs",
            "Sherlock",
            "Stranger Things 4",
            "Saw 3",
            "Se7en",
            "Severance",
            "Specials 2",
        ] {
            assert_eq!(parse_season_dir(name), None, "name: {name}");
        }
    }

    #[test]
    fn a_season_folder_with_a_label_keeps_the_name_the_user_gave_it() {
        // The part after the number is theirs and nobody else knows it. Printing
        // "Season 1" over it makes two folders they named differently look
        // identical in the list.
        for (name, expected) in [
            ("Season 1 - inizio", "Season 1 - inizio"),
            ("Stagione 2 — La vendetta", "Stagione 2 — La vendetta"),
            ("S03 - Finale", "S03 - Finale"),
            ("Season_4_The_End", "Season 4 The End"),
            ("Season.5.Rebirth", "Season 5 Rebirth"),
        ] {
            assert_eq!(
                season_dir_title(name).as_deref(),
                Some(expected),
                "name: {name}"
            );
        }
    }

    #[test]
    fn a_season_folder_that_only_states_its_number_has_no_name_of_its_own() {
        // Every one of these means the same thing, and the window says it more
        // clearly by building "Season N" from the number.
        for name in [
            "Season 2",
            "season 02",
            "SEASON  2",
            "S02",
            "s2",
            "Stagione 2",
            "Season_2",
            "Season.2",
            "Season-2",
            "Season2",
        ] {
            assert_eq!(season_dir_title(name), None, "name: {name}");
        }
    }

    #[test]
    fn season_zero_is_always_called_what_its_folder_is_called() {
        // There is no "Season 0" to fall back on, so the folder name is the
        // only thing there is to call it.
        assert_eq!(season_dir_title("Specials").as_deref(), Some("Specials"));
        assert_eq!(season_dir_title("extras").as_deref(), Some("extras"));
    }

    #[test]
    fn a_folder_the_fallback_turns_into_a_season_keeps_its_name() {
        // "Prima Stagione", "Parte 1": not recognised as season markers, turned
        // into seasons by position because they hold episodes. Their name is
        // the only thing anyone knows about them, so calling them "Season 1"
        // would be replacing information with a guess.
        for name in ["Prima Stagione", "Parte 1", "Behind the scenes"] {
            assert_eq!(
                season_dir_title(name).as_deref(),
                Some(name),
                "name: {name}"
            );
        }
        assert_eq!(season_dir_title(""), None);
    }

    #[test]
    fn parse_season_dir_maps_specials_to_zero() {
        assert_eq!(parse_season_dir("Specials"), Some(0));
        assert_eq!(parse_season_dir("extras"), Some(0));
    }

    #[test]
    fn parse_season_dir_rejects_non_seasons() {
        for name in [
            "Scrubs",
            "Season",
            "S01E02",
            "Behind the scenes",
            "Season two",
            "",
        ] {
            assert_eq!(parse_season_dir(name), None, "name: {name}");
        }
    }

    #[test]
    fn parse_episode_reads_sxxexx() {
        assert_eq!(
            parse_episode("Scrubs S01E02 - My Mentor"),
            Some(EpisodeRef {
                season: Some(1),
                episode: 2
            })
        );
        assert_eq!(
            parse_episode("scrubs s1e2"),
            Some(EpisodeRef {
                season: Some(1),
                episode: 2
            })
        );
    }

    #[test]
    fn parse_episode_reads_x_notation() {
        assert_eq!(
            parse_episode("Show 1x03"),
            Some(EpisodeRef {
                season: Some(1),
                episode: 3
            })
        );
    }

    #[test]
    fn parse_episode_reads_bare_markers() {
        assert_eq!(
            parse_episode("Ep 04 - Title"),
            Some(EpisodeRef {
                season: None,
                episode: 4
            })
        );
        assert_eq!(
            parse_episode("E05"),
            Some(EpisodeRef {
                season: None,
                episode: 5
            })
        );
        assert_eq!(
            parse_episode("Episodio 6"),
            Some(EpisodeRef {
                season: None,
                episode: 6
            })
        );
    }

    #[test]
    fn parse_episode_reads_leading_number() {
        assert_eq!(
            parse_episode("04 - Pilot"),
            Some(EpisodeRef {
                season: None,
                episode: 4
            })
        );
    }

    #[test]
    fn parse_episode_takes_the_first_marker_of_a_multi_episode_file() {
        assert_eq!(
            parse_episode("Show S01E02E03"),
            Some(EpisodeRef {
                season: Some(1),
                episode: 2
            })
        );
    }

    #[test]
    fn parse_episode_returns_none_without_a_marker() {
        for stem in ["Pilot", "My Mentor", "", "Blade Runner 2049"] {
            assert_eq!(parse_episode(stem), None, "stem: {stem}");
        }
    }

    #[test]
    fn episode_title_takes_the_text_after_the_marker() {
        assert_eq!(episode_title("Scrubs S01E02 - My Mentor"), "My Mentor");
        assert_eq!(episode_title("04 - Pilot"), "Pilot");
    }

    #[test]
    fn episode_title_is_empty_when_the_filename_is_only_a_marker() {
        assert_eq!(episode_title("S01E02"), "");
    }

    #[test]
    fn episode_title_falls_back_to_the_whole_stem_without_a_marker() {
        assert_eq!(episode_title("My Mentor"), "My Mentor");
    }

    #[test]
    fn natural_cmp_orders_digit_runs_numerically() {
        assert_eq!(natural_cmp("ep2", "ep10"), Ordering::Less);
        assert_eq!(natural_cmp("ep10", "ep2"), Ordering::Greater);
        assert_eq!(natural_cmp("ep02", "ep2"), Ordering::Equal);
    }

    #[test]
    fn natural_cmp_is_case_insensitive() {
        assert_eq!(natural_cmp("Ep2", "ep10"), Ordering::Less);
        assert_eq!(natural_cmp("alpha", "ALPHA"), Ordering::Equal);
    }

    #[test]
    fn natural_cmp_orders_plain_text() {
        assert_eq!(natural_cmp("alpha", "beta"), Ordering::Less);
        assert_eq!(natural_cmp("", "a"), Ordering::Less);
    }

    #[test]
    fn natural_cmp_handles_very_long_digit_runs() {
        // Longer than u64 — must not panic or wrap.
        assert_eq!(
            natural_cmp("f99999999999999999999999", "f100000000000000000000000"),
            Ordering::Less
        );
    }
}
