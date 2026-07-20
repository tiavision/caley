//! Static mapping from Windows/CLDR timezone display names (as commonly
//! emitted in the `TZID` parameter of Outlook/Exchange-produced `.ics`
//! files) to IANA (Olson) timezone identifiers usable by `chrono_tz`.
//!
//! Source: Unicode CLDR `windowsZones.xml`
//! (<https://github.com/unicode-org/cldr/blob/main/common/supplemental/windowsZones.xml>),
//! using the `territory="001"` ("golden") mapping for each Windows zone.
//! This is a curated subset covering the Windows zone names most commonly
//! seen in real-world calendar exports, not the full CLDR list. Extend by
//! pulling additional `mapZone` entries from `windowsZones.xml` as new
//! TZIDs are observed.

pub(crate) fn windows_to_iana(windows_name: &str) -> Option<&'static str> {
    WINDOWS_TZ_MAP
        .iter()
        .find(|(win, _)| *win == windows_name)
        .map(|(_, iana)| *iana)
}

static WINDOWS_TZ_MAP: &[(&str, &str)] = &[
    ("UTC", "Etc/UTC"),
    ("GMT Standard Time", "Europe/London"),
    ("Greenwich Standard Time", "Atlantic/Reykjavik"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Romance Standard Time", "Europe/Paris"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("FLE Standard Time", "Europe/Kyiv"),
    ("GTB Standard Time", "Europe/Bucharest"),
    ("E. Europe Standard Time", "Europe/Chisinau"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("Eastern Standard Time", "America/New_York"),
    ("Central Standard Time", "America/Chicago"),
    ("Mountain Standard Time", "America/Denver"),
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("US Eastern Standard Time", "America/Indianapolis"),
    ("Alaskan Standard Time", "America/Anchorage"),
    ("Hawaiian Standard Time", "Pacific/Honolulu"),
    ("Atlantic Standard Time", "America/Halifax"),
    ("SA Eastern Standard Time", "America/Cayenne"),
    ("SA Pacific Standard Time", "America/Bogota"),
    ("Pacific SA Standard Time", "America/Santiago"),
    ("E. South America Standard Time", "America/Sao_Paulo"),
    ("India Standard Time", "Asia/Calcutta"),
    ("China Standard Time", "Asia/Shanghai"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("Korea Standard Time", "Asia/Seoul"),
    ("Singapore Standard Time", "Asia/Singapore"),
    ("W. Australia Standard Time", "Australia/Perth"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("AUS Central Standard Time", "Australia/Darwin"),
    ("New Zealand Standard Time", "Pacific/Auckland"),
    ("SE Asia Standard Time", "Asia/Bangkok"),
    ("Arabian Standard Time", "Asia/Dubai"),
    ("Arab Standard Time", "Asia/Riyadh"),
    ("Israel Standard Time", "Asia/Jerusalem"),
    ("South Africa Standard Time", "Africa/Johannesburg"),
    ("Turkey Standard Time", "Europe/Istanbul"),
    ("Argentina Standard Time", "America/Buenos_Aires"),
    ("Central Brazilian Standard Time", "America/Cuiaba"),
    ("Newfoundland Standard Time", "America/St_Johns"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_common_windows_names() {
        assert_eq!(
            windows_to_iana("Eastern Standard Time"),
            Some("America/New_York")
        );
        assert_eq!(
            windows_to_iana("Romance Standard Time"),
            Some("Europe/Paris")
        );
        assert_eq!(windows_to_iana("Not A Real Zone"), None);
    }
}
