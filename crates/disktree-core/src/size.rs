//! Human-readable sizes and counts. Formatting never changes byte accounting.

use std::sync::atomic::{AtomicU8, Ordering};

const DECIMAL_UNITS: [&str; 7] = ["B", "kB", "MB", "GB", "TB", "PB", "EB"];
const BINARY_UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];

/// A presentation preference. Platform means the client OS, including when
/// displaying a scan from an SSH host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum SizeUnits {
    #[default]
    PlatformDefault,
    Decimal,
    Binary,
}

impl SizeUnits {
    pub const ALL: [Self; 3] =
        [Self::PlatformDefault, Self::Decimal, Self::Binary];

    pub const fn key(self) -> &'static str {
        match self {
            Self::PlatformDefault => "platform",
            Self::Decimal => "decimal",
            Self::Binary => "binary",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|units| units.key() == value)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::PlatformDefault => "Platform default",
            Self::Decimal => "Decimal",
            Self::Binary => "Binary",
        }
    }

    /// Explicit platform input lets tests cover every platform's default.
    #[must_use]
    pub const fn resolve_for_macos(self, macos: bool) -> Self {
        match self {
            Self::PlatformDefault if macos => Self::Decimal,
            Self::PlatformDefault => Self::Binary,
            explicit => explicit,
        }
    }

    #[must_use]
    pub const fn resolved(self) -> Self {
        self.resolve_for_macos(cfg!(target_os = "macos"))
    }

    const fn scale(self) -> (f64, &'static [&'static str; 7]) {
        match self.resolved() {
            Self::Decimal => (1000.0, &DECIMAL_UNITS),
            Self::Binary | Self::PlatformDefault => (1024.0, &BINARY_UNITS),
        }
    }

    /// One decimal below ten, whole numbers above; IEC labels for binary.
    pub fn format(self, bytes: u64) -> String {
        self.scaled(bytes).format(true)
    }

    /// Compact labels retain the unit's meaning, e.g. `1.5KiB` or `1.5kB`.
    pub fn format_short(self, bytes: u64) -> String {
        self.scaled(bytes).format(false)
    }

    /// Equal bounds appear once; both bounds always use the same preference.
    pub fn format_range(self, lower: u64, upper: u64) -> String {
        if lower == upper {
            self.format(lower)
        } else {
            format!("{}\u{2013}{}", self.format(lower), self.format(upper))
        }
    }

    fn scaled(self, bytes: u64) -> Scaled {
        let (base, labels) = self.scale();
        let mut value = bytes as f64;
        let mut unit = 0;
        // Promote where whole-number rounding would otherwise print the
        // base itself in the smaller unit (1000 kB or 1024 KiB).
        while value >= base - 0.5 && unit + 1 < labels.len() {
            value /= base;
            unit += 1;
        }
        Scaled {
            value,
            unit,
            labels,
        }
    }
}

// Library callers retain their historical default; application entry points
// initialize the saved preference before formatting anything. One atomic
// choice also covers worker-produced messages without touching their bytes.
static UNITS: AtomicU8 = AtomicU8::new(SizeUnits::Binary as u8);

pub fn units() -> SizeUnits {
    match UNITS.load(Ordering::Relaxed) {
        1 => SizeUnits::Decimal,
        2 => SizeUnits::Binary,
        _ => SizeUnits::PlatformDefault,
    }
}

pub fn set_units(units: SizeUnits) {
    UNITS.store(units as u8, Ordering::Relaxed);
}

/// Format with the application's active display preference.
pub fn human_bytes(bytes: u64) -> String {
    units().format(bytes)
}

pub fn human_bytes_range(lower: u64, upper: u64) -> String {
    units().format_range(lower, upper)
}

pub fn human_bytes_short(bytes: u64) -> String {
    units().format_short(bytes)
}

struct Scaled {
    value: f64,
    unit: usize,
    labels: &'static [&'static str; 7],
}

impl Scaled {
    fn format(&self, gap: bool) -> String {
        // Whole bytes have no fraction to show: `0 B`, not `0.0 B`.
        let text = if self.unit == 0 {
            format!("{:.0}", self.value)
        } else if self.value < 9.95 {
            format!("{:.1}", self.value)
        } else {
            format!("{:.0}", self.value)
        };
        let gap = if gap { " " } else { "" };
        format!("{text}{gap}{}", self.labels[self.unit])
    }
}

/// `1.2k`, `3.4M`, `812`: file counts, which grow past a million quickly.
pub fn human_count(count: u64) -> String {
    let value = count as f64;
    // Each bound sits where the one decimal would round up to 1000.0, so
    // 999,999 reads `1.0M`, not `1000.0k`.
    if count < 10_000 {
        format!("{count}")
    } else if value < 999_950.0 {
        format!("{:.1}k", value / 1_000.0)
    } else if value < 999_950_000.0 {
        format!("{:.1}M", value / 1_000_000.0)
    } else {
        format!("{:.1}G", value / 1_000_000_000.0)
    }
}

/// Share of `total` as a whole percentage, saturating instead of dividing by
/// zero.
pub fn share(part: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (part as f64 / total as f64) * 100.0
    }
}

/// Compass width for a share, e.g. `▓▓▓░░`.
pub fn share_bar(part: u64, total: u64, width: usize) -> String {
    let filled = if total == 0 {
        0
    } else {
        ((part as f64 / total as f64) * width as f64).round() as usize
    };
    let filled = filled.min(width);
    let mut bar = String::with_capacity(width);
    for index in 0..width {
        bar.push(if index < filled { '▓' } else { '░' });
    }
    bar
}

#[cfg(test)]
mod tests {
    use super::*;

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig {
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        // Powers and digits come from the unit definition, independently
        // of the formatter's scaling and rounding implementation.
        #[test]
        fn unit_definitions_determine_numbers_and_labels(
            binary in any::<bool>(), exponent in 1_u32..=5, digit in 1_u64..=9,
        ) {
            let (units, base, labels) = if binary {
                (SizeUnits::Binary, 1024_u64, ["B", "KiB", "MiB", "GiB", "TiB", "PiB"])
            } else {
                (SizeUnits::Decimal, 1000_u64, ["B", "kB", "MB", "GB", "TB", "PB"])
            };
            let bytes = digit * base.pow(exponent);
            let label = labels[exponent as usize];
            prop_assert_eq!(units.format(bytes), format!("{digit}.0 {label}"));
        }

        #[test]
        fn compact_and_range_formats_preserve_the_same_units(
            bytes in any::<u64>(), other in any::<u64>(), index in 0_usize..3,
        ) {
            let units = SizeUnits::ALL[index];
            let full = units.format(bytes);
            prop_assert_eq!(units.format_short(bytes), full.replace(' ', ""));
            prop_assert_eq!(&units.format_range(bytes, bytes), &full);
            if bytes != other {
                prop_assert_eq!(units.format_range(bytes, other),
                    format!("{full}–{}", units.format(other)));
            }
        }

        #[test]
        fn platform_only_changes_the_default(
            macos in any::<bool>(), bytes in any::<u64>(), index in 0_usize..3,
        ) {
            let preference = SizeUnits::ALL[index];
            let expected = match preference {
                SizeUnits::PlatformDefault if macos => SizeUnits::Decimal,
                SizeUnits::PlatformDefault => SizeUnits::Binary,
                explicit => explicit,
            };
            prop_assert_eq!(preference.resolve_for_macos(macos).format(bytes), expected.format(bytes));
        }

        // Parse the display back through the mathematical unit definition.
        // It may lose at most half a displayed increment to rounding.
        #[test]
        fn displayed_numbers_round_the_original_bytes(
            bytes in any::<u64>(), binary in any::<bool>(),
        ) {
            let (units, base, labels) = if binary {
                (SizeUnits::Binary, 1024_f64, BINARY_UNITS)
            } else {
                (SizeUnits::Decimal, 1000_f64, DECIMAL_UNITS)
            };
            let text = units.format(bytes);
            let (number, suffix) = text.split_once(' ').expect("number and unit");
            let exponent = labels.iter().position(|label| *label == suffix).expect("truthful label");
            let factor = (0..exponent).fold(1.0, |factor, _| factor * base);
            let shown = number.parse::<f64>().expect("number");
            let step = if number.contains('.') { 0.1 } else { 1.0 };
            let error = shown.mul_add(-factor, bytes as f64).abs();
            prop_assert!(error <= factor * step * 0.500_001);
            prop_assert!(shown < base || exponent == labels.len() - 1);
        }
    }

    #[test]
    fn application_preference_routes_every_formatter() {
        const CHILD: &str = "PWRDISK_SIZE_UNITS_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            // The active choice belongs to a whole application process.
            // Isolate its mutations from concurrently running legacy tests.
            let status = std::process::Command::new(
                std::env::current_exe().expect("test binary"),
            )
            .args([
                "--exact",
                "size::tests::application_preference_routes_every_formatter",
            ])
            .env(CHILD, "1")
            .status()
            .expect("formatter process");
            assert!(status.success());
            return;
        }
        for units in SizeUnits::ALL {
            set_units(units);
            for bytes in [0, 999, 1000, 1024, 1_000_000, 1_048_576, u64::MAX] {
                assert_eq!(human_bytes(bytes), units.format(bytes));
                assert_eq!(human_bytes_short(bytes), units.format_short(bytes));
                assert_eq!(
                    human_bytes_range(0, bytes),
                    units.format_range(0, bytes)
                );
            }
        }
    }

    #[test]
    fn decimal_and_binary_boundaries_use_truthful_labels() {
        assert_eq!(SizeUnits::Decimal.format(1000), "1.0 kB");
        assert_eq!(SizeUnits::Binary.format(1000), "1000 B");
        assert_eq!(SizeUnits::Decimal.format(1_000_000 - 1), "1.0 MB");
        assert_eq!(SizeUnits::Binary.format(1_048_576 - 1), "1.0 MiB");
        assert_eq!(SizeUnits::Decimal.format(u64::MAX), "18 EB");
        assert_eq!(SizeUnits::Binary.format(u64::MAX), "16 EiB");
    }

    #[test]
    fn bytes_use_binary_units_with_one_decimal_below_ten() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(7), "7 B");
        assert_eq!(human_bytes(12), "12 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(
            human_bytes(1024 * 1024 * 1024 + 400 * 1024 * 1024),
            "1.4 GiB"
        );
        assert_eq!(human_bytes(512 * 1024 * 1024), "512 MiB");
    }

    #[test]
    fn short_form_drops_the_space() {
        assert_eq!(human_bytes_short(1536), "1.5KiB");
        assert_eq!(human_bytes_short(512 * 1024), "512KiB");
    }

    #[test]
    fn counts_switch_to_metric_suffixes() {
        assert_eq!(human_count(0), "0");
        assert_eq!(human_count(9_999), "9999");
        assert_eq!(human_count(12_000), "12.0k");
        assert_eq!(human_count(2_500_000), "2.5M");
    }

    #[test]
    fn a_value_that_rounds_up_to_the_next_unit_is_shown_in_it() {
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024 * 1024 - 1), "1.0 MiB");
        assert_eq!(human_bytes(1024 * 1024 * 1024 - 1), "1.0 GiB");
        assert_eq!(human_bytes_short(1024 * 1024 - 1), "1.0MiB");
        assert_eq!(human_bytes(1023 * 1024), "1023 KiB");
        assert_eq!(human_count(999_949), "999.9k");
        assert_eq!(human_count(999_999), "1.0M");
        assert_eq!(human_count(999_999_999), "1.0G");
    }

    #[test]
    fn share_survives_a_zero_total() {
        assert!((share(5, 0) - 0.0).abs() < f64::EPSILON);
        assert!((share(1, 4) - 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn share_bar_is_always_the_requested_width() {
        assert_eq!(share_bar(1, 4, 4).chars().count(), 4);
        assert_eq!(share_bar(4, 4, 4), "▓▓▓▓");
        assert_eq!(share_bar(0, 4, 4), "░░░░");
        assert_eq!(share_bar(1, 0, 3).chars().count(), 3);
    }
}
