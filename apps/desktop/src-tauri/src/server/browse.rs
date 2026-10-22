//! `GET /api/browse/:page` (2026-10-22): portada y filas de Películas, Series e Inicio,
//! como en Netflix. Qué va en cada fila lo decide `catalog::browse` con la semilla de la
//! visita; aquí se reúnen los datos, se rellenan los títulos y se quita lo que el perfil no
//! puede ver (F14).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::catalog::browse::{self, Card, KindInputs, Plan, RowStyle};
use crate::catalog::normalize::current_year;
use crate::catalog::recommendations::Recommendations;
use crate::catalog::trending::Trending;
use crate::db::{now_epoch, reads, schema, DbError};

/// Insignia «Nuevo»: añadido en la última semana (igual que en el resto de la API).
const NEW_BADGE_WINDOW_SECONDS: i64 = 7 * 24 * 3600;

#[derive(Clone)]
pub struct BrowseState {
    pub db_path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
enum BrowseError {
    #[error("no encontrado")]
    NotFound,
    #[error("error de base de datos: {0}")]
    Db(#[from] DbError),
    #[error("error interno")]
    Internal,
}

impl IntoResponse for BrowseError {
    fn into_response(self) -> Response {
        let status = match self {
            BrowseError::NotFound => StatusCode::NOT_FOUND,
            BrowseError::Db(_) | BrowseError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}

pub fn browse_router(state: BrowseState) -> Router {
    Router::new().route("/api/browse/:page", get(get_browse)).with_state(state)
}

/// Un título en una fila.
#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct BrowseItem {
    /// `movie`, `series` o `episode` (solo en «Seguir viendo» de Series).
    pub kind: String,
    #[ts(type = "number")]
    pub id: i64,
    /// Serie del episodio, para abrir el reproductor con su lista de episodios.
    #[ts(type = "number | null")]
    pub series_id: Option<i64>,
    pub clean_title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    /// Segunda línea: por qué está ahí, el episodio, o lo que falta.
    pub subtitle: Option<String>,
    /// Cuánto se ha visto (0–1), en «Seguir viendo».
    pub progress: Option<f64>,
    pub is_new: bool,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct BrowseRowResponse {
    /// Estable entre visitas («top», «genre-28»...).
    pub key: String,
    pub title: String,
    /// `poster`, `top10` (con el número grande) o `continue` (abrir es reproducir).
    pub style: String,
    pub items: Vec<BrowseItem>,
}

/// La portada de la pantalla.
#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct BrowseFeatured {
    /// `movie` o `series`.
    pub kind: String,
    #[ts(type = "number")]
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub backdrop_path: Option<String>,
    pub poster_path: Option<String>,
    pub rating: Option<f64>,
    /// «N.º 2 en España hoy», «Recomendada para ti»...
    pub reason: String,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct BrowseResponse {
    pub featured: Option<BrowseFeatured>,
    pub rows: Vec<BrowseRowResponse>,
    /// Las recomendaciones del perfil se están calculando por primera vez: conviene volver a
    /// pedir la pantalla en unos segundos.
    pub pending: bool,
}

#[derive(Deserialize)]
struct BrowseQuery {
    profile_id: i64,
    /// Semilla de la visita: la misma da la misma pantalla.
    seed: Option<u64>,
    /// Portadas de las últimas visitas, para no repetirlas: `movie:12,series:4`.
    avoid: Option<String>,
}

fn parse_avoid(raw: Option<&str>) -> HashSet<(&'static str, i64)> {
    raw.unwrap_or_default()
        .split(',')
        .filter_map(|part| {
            let (kind, id) = part.trim().split_once(':')?;
            let kind: &'static str = match kind {
                "movie" => "movie",
                "series" => "series",
                _ => return None,
            };
            Some((kind, id.trim().parse().ok()?))
        })
        .collect()
}

/// Lo que se reúne de un tipo antes de planificar.
struct KindData {
    kind: &'static str,
    index: std::sync::Arc<Vec<reads::BrowseIndexRow>>,
    taste: browse::Taste,
    genre_names: HashMap<i64, String>,
    top: Vec<i64>,
    trending: Vec<i64>,
    recommended: Vec<(i64, String)>,
    because: Vec<(String, Vec<i64>)>,
    recent: Vec<i64>,
    my_list: Vec<i64>,
}

impl KindData {
    fn inputs(&self, now_year: i32) -> KindInputs<'_> {
        KindInputs {
            kind: self.kind,
            index: &self.index,
            taste: &self.taste,
            genre_names: &self.genre_names,
            top: &self.top,
            trending: &self.trending,
            recommended: &self.recommended,
            because: &self.because,
            recent: &self.recent,
            my_list: &self.my_list,
            now_year,
        }
    }
}

fn gather(conn: &rusqlite::Connection, kind: &'static str, profile_id: i64, recommendations: &Recommendations, trending: &Trending, now: i64) -> crate::db::Result<KindData> {
    let series = kind == "series";
    let signals = reads::list_taste_signals(conn, profile_id, kind)?;
    let genre_names = reads::list_genre_names(conn, if series { "tv" } else { "movie" })?.into_iter().map(|g| (g.id, g.name)).collect();
    let (top, trending_ids, recommended) = if series {
        (
            trending.top_series_es.iter().map(|t| t.id).collect(),
            trending.series.iter().map(|t| t.id).collect(),
            recommendations.series.iter().map(|r| (r.id, r.because_of.clone())).collect(),
        )
    } else {
        (
            trending.top_movies_es.iter().map(|t| t.id).collect(),
            trending.movies.iter().map(|t| t.id).collect(),
            recommendations.movies.iter().map(|r| (r.id, r.because_of.clone())).collect(),
        )
    };
    let because = recommendations
        .because
        .iter()
        .filter(|row| row.kind == kind)
        .map(|row| (row.seed_title.clone(), row.items.iter().map(|t| t.id).collect()))
        .collect();
    let my_list = reads::list_favorites(conn, profile_id)?.into_iter().filter(|f| f.item_kind == kind).map(|f| f.item_id).collect();
    Ok(KindData {
        kind,
        index: browse::cached_index(conn, kind)?,
        taste: browse::taste_from_signals(&signals, kind, now),
        genre_names,
        top,
        trending: trending_ids,
        recommended,
        because,
        recent: reads::recent_title_ids(conn, kind, 80)?,
        my_list,
    })
}

/// «Seguir viendo» de Películas (películas a medias) o de Series (episodios a medias y el
/// siguiente de cada serie empezada).
fn continue_row(conn: &rusqlite::Connection, kind: &str, profile_id: i64, access: reads::ProfileAccess) -> crate::db::Result<Option<BrowseRowResponse>> {
    let mut items = Vec::new();
    let mut series_seen = HashSet::new();
    for row in reads::list_continue_watching(conn, profile_id, 40)? {
        let progress = row.duration_s.filter(|d| *d > 0).map(|d| (row.position_s as f64 / d as f64).clamp(0.0, 1.0));
        if kind == "movie" && row.item_kind == "movie" && reads::item_allowed(conn, "movie", row.item_id, access)? {
            items.push(BrowseItem { kind: "movie".into(), id: row.item_id, series_id: None, clean_title: row.title, year: None, poster_path: row.poster_path, subtitle: None, progress, is_new: false });
        } else if kind == "series" && row.item_kind == "episode" && reads::item_allowed(conn, "episode", row.item_id, access)? {
            let series_id = reads::series_id_of_episode(conn, row.item_id)?;
            if let Some(id) = series_id {
                series_seen.insert(id);
            }
            items.push(BrowseItem { kind: "episode".into(), id: row.item_id, series_id, clean_title: row.title, year: None, poster_path: row.poster_path, subtitle: row.subtitle, progress, is_new: false });
        }
    }
    if kind == "series" {
        for next in reads::list_next_episodes(conn, profile_id, 40)? {
            if series_seen.contains(&next.series_id) || !reads::item_allowed(conn, "series", next.series_id, access)? {
                continue;
            }
            let subtitle = format!("T{} E{}{}", next.season, next.number, next.episode_title.map(|t| format!(" · {t}")).unwrap_or_default());
            items.push(BrowseItem {
                kind: "episode".into(),
                id: next.episode_id,
                series_id: Some(next.series_id),
                clean_title: next.series_title,
                year: None,
                poster_path: next.still_path.or(next.poster_path),
                subtitle: Some(subtitle),
                progress: None,
                is_new: false,
            });
        }
    }
    items.truncate(20);
    Ok((!items.is_empty()).then(|| BrowseRowResponse { key: "continue".into(), title: "Seguir viendo".into(), style: "continue".into(), items }))
}

/// Rellena el plan con títulos y portadas, quitando lo que el perfil no puede ver y las
/// filas que se quedan cortas.
fn hydrate(conn: &rusqlite::Connection, plan: Plan, access: reads::ProfileAccess, now: i64) -> crate::db::Result<BrowseResponse> {
    let mut wanted: HashMap<&'static str, Vec<i64>> = HashMap::new();
    for card in plan.rows.iter().flat_map(|r| r.cards.iter()) {
        wanted.entry(card.kind).or_default().push(card.id);
    }
    if let Some(f) = &plan.featured {
        wanted.entry(f.kind).or_default().push(f.id);
    }
    let mut cards_by_kind: HashMap<&'static str, HashMap<i64, reads::BrowseCardRow>> = HashMap::new();
    let mut allowed: HashSet<(&'static str, i64)> = HashSet::new();
    for (kind, ids) in wanted {
        let found = reads::browse_cards(conn, kind, &ids)?;
        for id in found.keys() {
            if reads::item_allowed(conn, kind, *id, access)? {
                allowed.insert((kind, *id));
            }
        }
        cards_by_kind.insert(kind, found);
    }
    let lookup = |kind: &'static str, id: i64| -> Option<&reads::BrowseCardRow> {
        if !allowed.contains(&(kind, id)) {
            return None;
        }
        cards_by_kind.get(kind)?.get(&id)
    };

    let featured = plan.featured.as_ref().and_then(|f| {
        let card = lookup(f.kind, f.id)?;
        Some(BrowseFeatured {
            kind: f.kind.to_string(),
            id: f.id,
            clean_title: card.clean_title.clone(),
            year: card.year,
            overview: card.overview.clone(),
            backdrop_path: card.backdrop_path.clone(),
            poster_path: card.poster_path.clone(),
            rating: card.rating,
            reason: f.reason.clone(),
        })
    });

    let item = |card: &Card| -> Option<BrowseItem> {
        let row = lookup(card.kind, card.id)?;
        Some(BrowseItem {
            kind: card.kind.to_string(),
            id: card.id,
            series_id: None,
            clean_title: row.clean_title.clone(),
            year: row.year,
            poster_path: row.poster_path.clone(),
            subtitle: card.reason.clone().or_else(|| row.year.map(|y| y.to_string())),
            progress: None,
            is_new: row.added_at.is_some_and(|t| now - t < NEW_BADGE_WINDOW_SECONDS),
        })
    };
    let rows = plan
        .rows
        .iter()
        .filter_map(|row| {
            let items: Vec<BrowseItem> = row.cards.iter().filter_map(item).collect();
            let min = match (row.style, row.key.as_str()) {
                (RowStyle::Top10, _) => 3,
                (_, "my-list") => 1,
                (_, key) if key.starts_with("because") => 4,
                _ => browse::MIN_ROW,
            };
            (items.len() >= min).then(|| BrowseRowResponse {
                key: row.key.clone(),
                title: row.title.clone(),
                style: if row.style == RowStyle::Top10 { "top10".into() } else { "poster".into() },
                items,
            })
        })
        .collect();
    Ok(BrowseResponse { featured, rows, pending: false })
}

async fn get_browse(State(state): State<BrowseState>, AxumPath(page): AxumPath<String>, Query(query): Query<BrowseQuery>) -> Result<Json<BrowseResponse>, BrowseError> {
    let kinds: &'static [&'static str] = match page.as_str() {
        "movies" => &["movie"],
        "series" => &["series"],
        "home" => &["movie", "series"],
        _ => return Err(BrowseError::NotFound),
    };
    let profile_id = query.profile_id;
    let seed = query.seed.unwrap_or(0);
    let avoid = parse_avoid(query.avoid.as_deref());
    // Ninguna de las dos espera a TMDB: lo que haya en caché (ver sus módulos).
    let recommendations = crate::catalog::recommendations::get_cached_or_refresh(state.db_path.clone(), profile_id).await;
    let trending = crate::catalog::trending::get_cached_or_refresh(state.db_path.clone()).await;
    let pending = crate::catalog::recommendations::first_computation_pending(profile_id).await;
    let db_path = state.db_path;
    let response = tokio::task::spawn_blocking(move || -> crate::db::Result<BrowseResponse> {
        let conn = schema::open_readonly_at(&db_path)?;
        let access = reads::profile_access(&conn, profile_id)?;
        let now = now_epoch();
        let now_year = current_year();
        let data: Vec<KindData> = kinds.iter().map(|kind| gather(&conn, kind, profile_id, &recommendations, &trending, now)).collect::<crate::db::Result<_>>()?;
        let plan = match data.as_slice() {
            [movies, series] => browse::plan_home(&movies.inputs(now_year), &series.inputs(now_year), seed, &avoid),
            [one] => browse::plan_kind(&one.inputs(now_year), seed, &avoid, false),
            _ => Plan::default(),
        };
        let mut response = hydrate(&conn, plan, access, now)?;
        response.pending = pending;
        if let [one] = data.as_slice() {
            if let Some(row) = continue_row(&conn, one.kind, profile_id, access)? {
                response.rows.insert(0, row);
            }
        }
        Ok(response)
    })
    .await
    .map_err(|_| BrowseError::Internal)??;
    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avoided_featured_titles_are_parsed_and_junk_is_ignored() {
        let avoid = parse_avoid(Some("movie:12, series:4,canal:9,movie:x,,series"));
        assert_eq!(avoid, [("movie", 12), ("series", 4)].into_iter().collect());
        assert!(parse_avoid(None).is_empty());
    }
}
