//! Where a report may come from, and how many reports become one pin.

/// Reports must come from inside this box: Singapore with its islands, and
/// a little sea around them.
const MIN_LAT: f64 = 1.15;
const MAX_LAT: f64 = 1.48;
const MIN_LNG: f64 = 103.6;
const MAX_LNG: f64 = 104.1;

pub fn in_singapore(lat: f64, lng: f64) -> bool {
    (MIN_LAT..=MAX_LAT).contains(&lat) && (MIN_LNG..=MAX_LNG).contains(&lng)
}

/// The middle value, or the mean of the two middle ones. A pin sits at the
/// median of its reports' latitudes and longitudes, so one report with a bad
/// GPS fix can't drag it away once others agree. None for no values.
pub fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    Some(if values.len() % 2 == 1 { values[mid] } else { (values[mid - 1] + values[mid]) / 2.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_of_odd_and_even_counts() {
        assert_eq!(median(&mut []), None);
        assert_eq!(median(&mut [1.3]), Some(1.3));
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), Some(2.5));
    }

    #[test]
    fn one_bad_fix_cannot_move_an_agreed_pin() {
        let mut lats = [1.30001, 1.30002, 1.42, 1.30003];
        let pin = median(&mut lats).unwrap();
        assert!((pin - 1.300025).abs() < 1e-9, "{pin}");
    }

    #[test]
    fn singapore_and_not() {
        assert!(in_singapore(1.2840, 103.8514));
        assert!(!in_singapore(1.5, 103.8)); // Johor Bahru
        assert!(!in_singapore(0.0, 0.0));
        assert!(!in_singapore(f64::NAN, 103.8));
    }
}
