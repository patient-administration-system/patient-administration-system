//! rtt repository — RTTPathway and RTTClockEvent

use crate::db::sql::{Conn, Stmt, Table};
use uuid::Uuid;

use crate::db::entities::{rtt_clock_event, rtt_pathway};
use crate::models::rtt::{RTTClockEvent, RTTEventKind, RTTPathway, RTTStatus};
use crate::{Error, Result};

pub struct RttRepository;

impl RttRepository {
    pub async fn create_pathway<C: Conn>(conn: &C, p: &RTTPathway) -> Result<RTTPathway> {
        let am = pathway_to_row(p)?;
        am.insert(conn).await?;
        Ok(p.clone())
    }

    pub async fn find_pathway_by_id<C: Conn>(conn: &C, id: Uuid) -> Result<Option<RTTPathway>> {
        let m = conn
            .one_as::<rtt_pathway::Model>(Stmt::new(
                "SELECT * FROM rtt_pathways WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?;
        m.map(pathway_from_model).transpose()
    }

    pub async fn list_pathways_by_patient<C: Conn>(
        conn: &C,
        person_id: Uuid,
    ) -> Result<Vec<RTTPathway>> {
        let rows = conn
            .all_as::<rtt_pathway::Model>(Stmt::new(
                "SELECT * FROM rtt_pathways WHERE person_id = $1",
                [person_id.into()],
            ))
            .await?;
        rows.into_iter().map(pathway_from_model).collect()
    }

    pub async fn create_event<C: Conn>(conn: &C, e: &RTTClockEvent) -> Result<RTTClockEvent> {
        let am = event_to_row(e);
        am.insert(conn).await?;
        Ok(e.clone())
    }

    pub async fn list_events_for_pathway<C: Conn>(
        conn: &C,
        pathway_id: Uuid,
    ) -> Result<Vec<RTTClockEvent>> {
        let rows = conn
            .all_as::<rtt_clock_event::Model>(Stmt::new(
                "SELECT * FROM rtt_clock_events WHERE pathway_id = $1 ORDER BY event_at",
                [pathway_id.into()],
            ))
            .await?;
        rows.into_iter().map(event_from_model).collect()
    }
}

// --- conversion helpers ---

pub(crate) fn rtt_status_to_str(s: RTTStatus) -> &'static str {
    match s {
        RTTStatus::Active => "active",
        RTTStatus::Paused => "paused",
        RTTStatus::Stopped => "stopped",
    }
}

pub(crate) fn rtt_status_from_str(s: &str) -> Result<RTTStatus> {
    match s {
        "active" => Ok(RTTStatus::Active),
        "paused" => Ok(RTTStatus::Paused),
        "stopped" => Ok(RTTStatus::Stopped),
        other => Err(Error::internal(format!("unknown rtt status: {other}"))),
    }
}

pub(crate) fn rtt_event_kind_to_str(k: RTTEventKind) -> &'static str {
    match k {
        RTTEventKind::Started => "started",
        RTTEventKind::Paused => "paused",
        RTTEventKind::Resumed => "resumed",
        RTTEventKind::Stopped => "stopped",
    }
}

pub(crate) fn rtt_event_kind_from_str(s: &str) -> Result<RTTEventKind> {
    match s {
        "started" => Ok(RTTEventKind::Started),
        "paused" => Ok(RTTEventKind::Paused),
        "resumed" => Ok(RTTEventKind::Resumed),
        "stopped" => Ok(RTTEventKind::Stopped),
        other => Err(Error::internal(format!("unknown rtt event kind: {other}"))),
    }
}

fn pathway_to_row(p: &RTTPathway) -> Result<rtt_pathway::Model> {
    let breach_weeks_i32: i32 = p.breach_weeks.try_into().map_err(|_| {
        Error::internal(format!(
            "breach_weeks {} does not fit in i32",
            p.breach_weeks
        ))
    })?;
    Ok(rtt_pathway::Model {
        id: p.id,
        person_id: p.person_id,
        target_service: p.target_service.clone(),
        breach_weeks: breach_weeks_i32,
        status: rtt_status_to_str(p.status).to_string(),
        started_at: p.started_at.fixed_offset(),
        stopped_at: p.stopped_at.map(|t| t.fixed_offset()),
        created_at: p.created_at.fixed_offset(),
        updated_at: p.updated_at.fixed_offset(),
    })
}

fn pathway_from_model(m: rtt_pathway::Model) -> Result<RTTPathway> {
    let breach_weeks: u32 = m.breach_weeks.try_into().map_err(|_| {
        Error::internal(format!(
            "breach_weeks {} is negative; cannot convert to u32",
            m.breach_weeks
        ))
    })?;
    Ok(RTTPathway {
        id: m.id,
        person_id: m.person_id,
        target_service: m.target_service,
        breach_weeks,
        status: rtt_status_from_str(&m.status)?,
        started_at: m.started_at.with_timezone(&chrono::Utc),
        stopped_at: m.stopped_at.map(|t| t.with_timezone(&chrono::Utc)),
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

fn event_to_row(e: &RTTClockEvent) -> rtt_clock_event::Model {
    rtt_clock_event::Model {
        id: e.id,
        pathway_id: e.pathway_id,
        kind: rtt_event_kind_to_str(e.kind).to_string(),
        reason: e.reason.clone(),
        event_at: e.event_at.fixed_offset(),
        created_at: e.created_at.fixed_offset(),
    }
}

fn event_from_model(m: rtt_clock_event::Model) -> Result<RTTClockEvent> {
    Ok(RTTClockEvent {
        id: m.id,
        pathway_id: m.pathway_id,
        kind: rtt_event_kind_from_str(&m.kind)?,
        reason: m.reason,
        event_at: m.event_at.with_timezone(&chrono::Utc),
        created_at: m.created_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rtt_pathway_roundtrip_via_row() {
        let p = RTTPathway::new(Uuid::new_v4(), "cardiology");
        let am = pathway_to_row(&p).expect("to active model");
        let m = rtt_pathway::Model {
            id: am.id,
            person_id: am.person_id,
            target_service: am.target_service.clone(),
            breach_weeks: am.breach_weeks,
            status: am.status.clone(),
            started_at: am.started_at,
            stopped_at: am.stopped_at,
            created_at: am.created_at,
            updated_at: am.updated_at,
        };
        let back = pathway_from_model(m).expect("from model");
        assert_eq!(back.id, p.id);
        assert_eq!(back.breach_weeks, 18);
        assert_eq!(back.status, RTTStatus::Active);
        assert_eq!(back.target_service, "cardiology");
    }
}
