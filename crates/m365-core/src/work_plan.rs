//! Microsoft 365 work-plan helpers used by ntfy work-day forwarding.

use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;

use crate::graph::GraphClient;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkPlanOccurrence {
    #[serde(default)]
    work_location_type: Option<String>,
}

fn occurrence_kind_is_working(kind: Option<&str>) -> bool {
    kind.is_some_and(|kind| {
        kind.eq_ignore_ascii_case("office")
            || kind.eq_ignore_ascii_case("remote")
            || kind.eq_ignore_ascii_case("unspecified")
    })
}

fn occurrences_mean_working_now(occurrences: &[WorkPlanOccurrence]) -> bool {
    if occurrences.iter().any(|occurrence| {
        occurrence
            .work_location_type
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("timeOff"))
    }) {
        return false;
    }

    occurrences
        .iter()
        .any(|occurrence| occurrence_kind_is_working(occurrence.work_location_type.as_deref()))
}

fn occurrences_view_path(now: DateTime<Utc>) -> String {
    let end = now + chrono::Duration::seconds(1);
    let start = now.to_rfc3339_opts(SecondsFormat::Secs, true);
    let end = end.to_rfc3339_opts(SecondsFormat::Secs, true);

    format!(
        "me/settings/workHoursAndLocations/occurrencesView(startDateTime='{start}',endDateTime='{end}')?$select=workLocationType&$top=20"
    )
}

/// Return whether Microsoft 365 says the current instant is inside the user's
/// work plan. A time-off occurrence takes precedence over a working occurrence.
pub async fn working_now(graph: &GraphClient, now: DateTime<Utc>) -> Result<bool> {
    let occurrences: Vec<WorkPlanOccurrence> = graph.get_page(&occurrences_view_path(now)).await?;
    Ok(occurrences_mean_working_now(&occurrences))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn occurrence(kind: &str) -> WorkPlanOccurrence {
        WorkPlanOccurrence {
            work_location_type: Some(kind.to_string()),
        }
    }

    #[test]
    fn working_types_are_recognized() {
        assert!(occurrences_mean_working_now(&[occurrence("office")]));
        assert!(occurrences_mean_working_now(&[occurrence("remote")]));
        assert!(occurrences_mean_working_now(&[occurrence("unspecified")]));
        assert!(!occurrences_mean_working_now(&[]));
        assert!(!occurrences_mean_working_now(&[occurrence(
            "unknownFutureValue"
        )]));
    }

    #[test]
    fn time_off_overrides_working_occurrences() {
        assert!(!occurrences_mean_working_now(&[
            occurrence("remote"),
            occurrence("timeOff"),
        ]));
    }

    #[test]
    fn path_uses_a_one_second_utc_window() {
        let now = Utc.with_ymd_and_hms(2026, 9, 22, 8, 30, 45).unwrap();
        assert_eq!(
            occurrences_view_path(now),
            "me/settings/workHoursAndLocations/occurrencesView(startDateTime='2026-09-22T08:30:45Z',endDateTime='2026-09-22T08:30:46Z')?$select=workLocationType&$top=20"
        );
    }
}
