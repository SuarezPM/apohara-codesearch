// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Unit conversions for temperature and length. Deliberately uses bare formula
// vocabulary (no "convert"/"celsius" in some bodies) so a natural-language query
// like "centigrade to fahrenheit" becomes a hard, known-miss case for lexical
// search — the identifiers and bodies do not contain the word "centigrade".

/// Convert a temperature from Celsius to Fahrenheit.
pub fn celsius_to_fahrenheit(c: f64) -> f64 {
    c * 9.0 / 5.0 + 32.0
}

/// Convert a temperature from Fahrenheit to Celsius.
pub fn fahrenheit_to_celsius(f: f64) -> f64 {
    (f - 32.0) * 5.0 / 9.0
}

/// Convert a temperature from Celsius to Kelvin.
pub fn celsius_to_kelvin(c: f64) -> f64 {
    c + 273.15
}

/// Convert a length from miles to kilometers.
pub fn miles_to_kilometers(miles: f64) -> f64 {
    miles * 1.609_344
}

/// Convert a length from kilometers to miles.
pub fn kilometers_to_miles(km: f64) -> f64 {
    km / 1.609_344
}
