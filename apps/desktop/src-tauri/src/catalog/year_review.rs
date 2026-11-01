//! «Tu año en VIBE» (F17, parte 5): el resumen del año de un perfil, al estilo Spotify
//! Wrapped. Sale del tiempo visto (`watch_time`, desde la parte 1, de todos los aparatos
//! del perfil cuando viaja con la cuenta) y, para lo de antes, de lo terminado
//! (`playback_state`): una estimación con su duración, que la pantalla marca como tal.

use std::collections::HashMap;

use rusqlite::{params, Connection};

use crate::db::{reads, Result};

/// Un título del resumen: cuánto se vio y, si sigue en el catálogo, su portada.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewTitle {
    pub kind: String,
    pub key: String,
    pub title: String,
    pub seconds: i64,
    pub id: Option<i64>,
    pub poster_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct YearReview {
    pub year: i32,
    pub total_seconds: i64,
    /// Lo que sale de lo terminado antes de que se apuntara el tiempo (estimado).
    pub estimated_seconds: i64,
    pub movie_seconds: i64,
    pub series_seconds: i64,
    pub live_seconds: i64,
    pub movies_watched: i64,
    pub episodes_watched: i64,
    pub days_watched: i64,
    pub longest_streak: i64,
    pub busiest_day: Option<(String, i64)>,
    /// 0-23.
    pub favorite_hour: Option<i64>,
    /// 0 lunes … 6 domingo.
    pub favorite_weekday: Option<i64>,
    pub top_series: Vec<ReviewTitle>,
    pub top_movies: Vec<ReviewTitle>,
    pub top_channels: Vec<ReviewTitle>,
    /// Géneros con más tiempo: nombre y segundos.
    pub top_genres: Vec<(String, i64)>,
    pub first_title: Option<ReviewTitle>,
}

/// Días desde 1970 de una fecha `AAAA-MM-DD`.
fn days_of(day: &str) -> Option<i64> {
    let mut parts = day.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    // Inversa de `watch_time::civil_from_days` (algoritmo de Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// Racha más larga de días seguidos, de una lista de fechas `AAAA-MM-DD`.
fn longest_streak(days: &[String]) -> i64 {
    let mut numbers: Vec<i64> = days.iter().filter_map(|d| days_of(d)).collect();
    numbers.sort_unstable();
    numbers.dedup();
    let (mut best, mut run) = (0, 0);
    let mut previous: Option<i64> = None;
    for day in numbers {
        run = if previous == Some(day - 1) { run + 1 } else { 1 };
        best = best.max(run);
        previous = Some(day);
    }
    best
}

/// La serie de un episodio por su clave (`serie:temporada:número`).
fn series_key(item_key: &str) -> &str {
    item_key.split(':').next().unwrap_or(item_key)
}

fn top(map: HashMap<(String, String), (String, i64)>, limit: usize) -> Vec<ReviewTitle> {
    let mut list: Vec<ReviewTitle> = map.into_iter().map(|((kind, key), (title, seconds))| ReviewTitle { kind, key, title, seconds, id: None, poster_path: None }).collect();
    list.sort_by(|a, b| b.seconds.cmp(&a.seconds).then_with(|| a.title.cmp(&b.title)));
    list.truncate(limit);
    list
}

/// Portada e id local de un título del resumen, si sigue en el catálogo.
fn attach_card(conn: &Connection, title: &mut ReviewTitle) -> Result<()> {
    let kind = if title.kind == "series" { "series" } else { "movie" };
    if let Some(id) = reads::item_id_for_key(conn, kind, &title.key)? {
        if let Some(card) = reads::browse_cards(conn, kind, &[id])?.remove(&id) {
            title.poster_path = card.poster_path;
        }
        title.id = Some(id);
    }
    Ok(())
}

/// El resumen de un año de un perfil.
pub fn year_review(conn: &Connection, profile_id: i64, year: i32) -> Result<YearReview> {
    let prefix = format!("{year:04}-%");
    let mut stmt = conn.prepare("SELECT day, hour, item_kind, item_key, title, seconds FROM watch_time WHERE profile_id = ?1 AND day LIKE ?2")?;
    let rows = stmt
        .query_map(params![profile_id, prefix], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, i64>(5)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut review = YearReview { year, ..Default::default() };
    let mut by_day: HashMap<String, i64> = HashMap::new();
    let mut by_hour = [0i64; 24];
    let mut by_weekday = [0i64; 7];
    let mut series: HashMap<(String, String), (String, i64)> = HashMap::new();
    let mut movies: HashMap<(String, String), (String, i64)> = HashMap::new();
    let mut channels: HashMap<(String, String), (String, i64)> = HashMap::new();
    let mut first: Option<(String, i64, ReviewTitle)> = None;
    let mut seen_episodes = std::collections::HashSet::new();
    let mut seen_movies = std::collections::HashSet::new();

    for (day, hour, kind, key, title, seconds) in rows {
        review.total_seconds += seconds;
        *by_day.entry(day.clone()).or_default() += seconds;
        if (0..24).contains(&hour) {
            by_hour[hour as usize] += seconds;
        }
        if let Some(days) = days_of(&day) {
            // 1970-01-01 fue jueves (3 si el lunes es 0).
            by_weekday[((days + 3).rem_euclid(7)) as usize] += seconds;
        }
        let entry_title = match kind.as_str() {
            "movie" => {
                review.movie_seconds += seconds;
                seen_movies.insert(key.clone());
                movies.entry(("movie".into(), key.clone())).or_insert((title.clone(), 0)).1 += seconds;
                Some(("movie", key.clone()))
            }
            "episode" => {
                review.series_seconds += seconds;
                seen_episodes.insert(key.clone());
                let series_key = series_key(&key).to_string();
                series.entry(("series".into(), series_key.clone())).or_insert((title.clone(), 0)).1 += seconds;
                Some(("series", series_key))
            }
            "channel" => {
                review.live_seconds += seconds;
                channels.entry(("channel".into(), key.clone())).or_insert((title.clone(), 0)).1 += seconds;
                None
            }
            _ => None,
        };
        if let Some((kind, key)) = entry_title {
            let when = (day.clone(), hour);
            if first.as_ref().is_none_or(|(d, h, _)| (d.as_str(), *h) > (when.0.as_str(), when.1)) {
                first = Some((when.0, when.1, ReviewTitle { kind: kind.into(), key, title: title.clone(), seconds: 0, id: None, poster_path: None }));
            }
        }
    }

    // Lo terminado antes de que se apuntara el tiempo: su duración, como estimación.
    let start = days_of(&format!("{year:04}-01-01")).unwrap_or(0) * 86_400;
    let end = days_of(&format!("{:04}-01-01", year + 1)).unwrap_or(0) * 86_400;
    let mut finished = conn.prepare(
        "SELECT item_kind, item_id, COALESCE(duration_s, 0), played_at FROM playback_state
         WHERE profile_id = ?1 AND finished = 1 AND item_kind IN ('movie','episode') AND played_at >= ?2 AND played_at < ?3",
    )?;
    let done = finished
        .query_map(params![profile_id, start, end], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (kind, id, duration, played_at) in done {
        let Some(key) = reads::item_key_for(conn, &kind, id)? else { continue };
        let counted = if kind == "movie" { seen_movies.contains(&key) } else { seen_episodes.contains(&key) };
        if counted || duration <= 0 {
            continue;
        }
        let duration = duration.min(4 * 3600);
        review.total_seconds += duration;
        review.estimated_seconds += duration;
        let day = super::watch_time::local_day_hour(played_at, 0).0;
        *by_day.entry(day).or_default() += duration;
        if kind == "movie" {
            review.movie_seconds += duration;
            seen_movies.insert(key.clone());
            let title: String = conn.query_row("SELECT clean_title FROM movies WHERE id = ?1", params![id], |r| r.get(0)).unwrap_or_default();
            movies.entry(("movie".into(), key)).or_insert((title, 0)).1 += duration;
        } else {
            review.series_seconds += duration;
            seen_episodes.insert(key.clone());
            let series_title: String = conn
                .query_row(
                    "SELECT s.clean_title FROM episodes e JOIN series_sources ss ON ss.id = e.series_source_id JOIN series s ON s.id = ss.series_id WHERE e.id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .unwrap_or_default();
            series.entry(("series".into(), series_key(&key).to_string())).or_insert((series_title, 0)).1 += duration;
        }
    }

    review.movies_watched = seen_movies.len() as i64;
    review.episodes_watched = seen_episodes.len() as i64;
    review.days_watched = by_day.values().filter(|s| **s > 0).count() as i64;
    let days: Vec<String> = by_day.iter().filter(|(_, s)| **s > 0).map(|(d, _)| d.clone()).collect();
    review.longest_streak = longest_streak(&days);
    review.busiest_day = by_day.into_iter().max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)));
    review.favorite_hour = (0..24).filter(|h| by_hour[*h] > 0).max_by_key(|h| by_hour[*h]).map(|h| h as i64);
    review.favorite_weekday = (0..7).filter(|d| by_weekday[*d] > 0).max_by_key(|d| by_weekday[*d]).map(|d| d as i64);

    // Géneros: por el tiempo de cada película y serie, según su ficha del catálogo.
    let mut genre_seconds: HashMap<i64, i64> = HashMap::new();
    for ((kind, key), (_, seconds)) in movies.iter().chain(series.iter()) {
        let table_kind = if kind == "series" { "series" } else { "movie" };
        let Some(id) = reads::item_id_for_key(conn, table_kind, key)? else { continue };
        if let Some(card) = reads::browse_cards(conn, table_kind, &[id])?.remove(&id) {
            for genre in card.genres {
                *genre_seconds.entry(genre).or_default() += seconds;
            }
        }
    }
    let names: HashMap<i64, String> = {
        let mut stmt = conn.prepare("SELECT tmdb_genre_id, name FROM genre_names")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().collect()
    };
    let mut genres: Vec<(String, i64)> = genre_seconds.into_iter().filter_map(|(id, s)| names.get(&id).map(|n| (n.clone(), s))).collect();
    genres.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    // Un género con nombre en películas y en series se junta.
    let mut merged: Vec<(String, i64)> = Vec::new();
    for (name, seconds) in genres {
        match merged.iter_mut().find(|(n, _)| *n == name) {
            Some(existing) => existing.1 += seconds,
            None => merged.push((name, seconds)),
        }
    }
    merged.truncate(5);
    review.top_genres = merged;

    review.top_series = top(series, 5);
    review.top_movies = top(movies, 5);
    review.top_channels = top(channels, 3);
    for title in review.top_series.iter_mut().chain(review.top_movies.iter_mut()) {
        attach_card(conn, title)?;
    }
    if let Some((_, _, mut title)) = first {
        attach_card(conn, &mut title)?;
        review.first_title = Some(title);
    }
    Ok(review)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_streaks() {
        assert_eq!(days_of("1970-01-01"), Some(0));
        assert_eq!(days_of("2000-03-01"), Some(11_017));
        let days: Vec<String> = ["2026-01-30", "2026-01-31", "2026-02-01", "2026-02-03", "2026-02-01"].iter().map(|d| d.to_string()).collect();
        assert_eq!(longest_streak(&days), 3, "cruza el cambio de mes");
        assert_eq!(longest_streak(&[]), 0);
    }

    #[test]
    fn a_year_adds_up_by_series_movie_channel_and_moment() {
        let conn = crate::db::schema::open_connection_at(std::path::Path::new(":memory:")).unwrap();
        conn.execute_batch(
            "INSERT INTO providers (id, name, kind, base_url, created_at) VALUES (1, 'P', 'xtream', 'http://x', 0);
             INSERT INTO profiles (id, name, created_at) VALUES (1, 'Ana', 0);
             INSERT INTO movies (id, clean_title, match_key, genres) VALUES (1, 'Matrix', 'matrix-1999', '[878]');
             INSERT INTO movies (id, clean_title, match_key, genres) VALUES (2, 'Vieja', 'vieja-2000', '[18]');
             INSERT INTO series (id, clean_title, match_key, genres) VALUES (5, 'Serie', 'serie-2020', '[18]');
             INSERT INTO genre_names (kind, tmdb_genre_id, name) VALUES ('movie', 878, 'Ciencia ficción'), ('movie', 18, 'Drama');
             INSERT INTO watch_time (profile_id, device_id, day, hour, item_kind, item_key, title, seconds, updated_at) VALUES
               (1, 'pc', '2026-03-02', 22, 'movie', 'matrix-1999', 'Matrix', 7200, 0),
               (1, 'pc', '2026-03-03', 22, 'episode', 'serie-2020:1:1', 'Serie', 3000, 0),
               (1, 'movil', '2026-03-04', 8, 'episode', 'serie-2020:1:2', 'Serie', 3000, 0),
               (1, 'pc', '2026-03-04', 21, 'channel', 'La 1', 'La 1', 1800, 0),
               (1, 'pc', '2025-12-31', 23, 'movie', 'matrix-1999', 'Matrix', 999, 0);",
        )
        .unwrap();
        // Una película terminada en enero, antes de que se apuntara el tiempo: estimada.
        let jan = days_of("2026-01-10").unwrap() * 86_400;
        conn.execute("INSERT INTO playback_state (profile_id, item_kind, item_id, position_s, duration_s, finished, played_at) VALUES (1, 'movie', 2, 5400, 5400, 1, ?1)", params![jan]).unwrap();

        let review = year_review(&conn, 1, 2026).unwrap();
        assert_eq!(review.total_seconds, 7200 + 3000 + 3000 + 1800 + 5400, "el año pasado no cuenta");
        assert_eq!(review.estimated_seconds, 5400);
        assert_eq!((review.movies_watched, review.episodes_watched), (2, 2));
        assert_eq!(review.top_series[0].title, "Serie");
        assert_eq!(review.top_series[0].seconds, 6000, "lo visto en el móvil también suma");
        assert_eq!(review.top_series[0].id, Some(5));
        assert_eq!(review.top_movies.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), vec!["Matrix", "Vieja"]);
        assert_eq!(review.top_channels[0].title, "La 1");
        assert_eq!(review.longest_streak, 3);
        assert_eq!(review.favorite_hour, Some(22));
        assert_eq!(review.top_genres[0].0, "Drama", "serie y película vieja suman más que Matrix");
        assert_eq!(review.first_title.as_ref().map(|t| t.title.as_str()), Some("Matrix"));
        assert_eq!(review.busiest_day.as_ref().map(|(d, _)| d.as_str()), Some("2026-03-02"));
    }
}
