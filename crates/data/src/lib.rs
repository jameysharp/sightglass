//! Common data definitions for sightglass.
//!
//! These are in one place, pulled out from the rest of the crates, so that many
//! different crates can serialize and deserialize data by using the same
//! definitions.

#![deny(missing_docs, missing_debug_implementations)]

mod format;
pub use format::Format;

use serde::{Deserialize, Serialize};
use std::{borrow::Cow, str::FromStr};

/// A single measurement, for example instructions retired when compiling a Wasm
/// module.
///
/// This is often used with the `'static` lifetime when recording measurements,
/// where we can use string literals for various fields. When reading data, it
/// can be used with a non-static lifetime to avoid many small allocations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Measurement<'a> {
    /// The CPU architecture on which this measurement was taken, for example
    /// "aarch64" or "x86_64".
    pub arch: Cow<'a, str>,

    /// The file path of the wasmtime benchmark API shared library used to
    /// record this measurement.
    pub engine: Cow<'a, str>,

    /// The flags passed to the wasmtime benchmark API shared library for this
    /// measurement.
    pub engine_flags: Cow<'a, str>,

    /// The file path of the Wasm benchmark program.
    pub wasm: Cow<'a, str>,

    /// The id of the process within which this measurement was taken.
    pub process: u32,

    /// This measurement was the `n`th measurement of this phase taken within a
    /// process.
    pub iteration: u32,

    /// The phase in a Wasm program's lifecycle that was measured: compilation,
    /// instantiation, or execution.
    pub phase: Phase,

    /// The event that was measured: micro seconds of wall time, CPU cycles
    /// executed, instructions retired, cache misses, etc.
    pub event: Cow<'a, str>,

    /// The event counts.
    ///
    /// The meaning and units depend on what the `event` is: it might be a count
    /// of microseconds if the event is wall time, or it might be a count of
    /// instructions if the event is instructions retired.
    pub count: u64,
}

impl Measurement<'_> {
    /// The combination of engine and flags used for this measurement.
    pub fn engine_and_flags(&self) -> (&str, &str) {
        (&self.engine, &self.engine_flags)
    }
}

/// A phase in a Wasm program's lifecycle.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialOrd, Ord, PartialEq, Eq, Hash)]
pub enum Phase {
    /// The compilation phase, where Wasm bytes are translated into native
    /// machine code.
    Compilation,
    /// The instantiation phase, where imports are provided and memories,
    /// globals, and tables are initialized.
    Instantiation,
    /// The execution phase, where functions are called and instructions are
    /// executed.
    Execution,
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Phase::Compilation => write!(f, "compilation"),
            Phase::Instantiation => write!(f, "instantiation"),
            Phase::Execution => write!(f, "execution"),
        }
    }
}

impl FromStr for Phase {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_ascii_lowercase();
        match s.as_str() {
            "compilation" => Ok(Self::Compilation),
            "instantiation" => Ok(Self::Instantiation),
            "execution" => Ok(Self::Execution),
            _ => Err("invalid phase".into()),
        }
    }
}

/// A summary of grouped measurements.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Summary<'a> {
    /// The CPU architecture on which this measurement was taken, for example
    /// "aarch64" or "x86_64".
    pub arch: Cow<'a, str>,

    /// The file path of the wasmtime benchmark API shared library used to
    /// record this measurement.
    pub engine: Cow<'a, str>,

    /// The flags passed to the wasmtime benchmark API shared library for this
    /// measurement.
    pub engine_flags: Cow<'a, str>,

    /// The file path of the Wasm benchmark program.
    pub wasm: Cow<'a, str>,

    /// The phase in a Wasm program's lifecycle that was measured: compilation,
    /// instantiation, or execution.
    pub phase: Phase,

    /// The event that was measured: micro seconds of wall time, CPU cycles
    /// executed, instructions retired, cache misses, etc.
    pub event: Cow<'a, str>,

    /// The minimum value of the `count` field.
    pub min: u64,

    /// The maximum value of the `count` field.
    pub max: u64,

    /// The median value of the `count` field.
    pub median: u64,

    /// The arithmetic mean of the `count` field.
    pub mean: f64,

    /// The mean deviation (note: not standard deviation) of the `count` field.
    pub mean_deviation: f64,
}

/// One of the engines measured in [`EffectSize`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EngineResult {
    /// The engine being compared.
    ///
    /// This is the file path of the wasmtime benchmark API shared library used
    /// to record this measurement.
    pub engine: String,

    /// Flags used with this engine.
    pub engine_flags: String,

    /// The engine's result's arithmetic mean of the `count` field.
    pub mean: f64,
}

/// The effect size (and confidence interval) between two different engines
/// (i.e. two different commits of Wasmtime).
///
/// This allows us to justify statements like "we are 99% confident that the new
/// register allocator is 13.6% faster (± 1.7%) than the old register
/// allocator."
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EffectSize<'a> {
    /// The CPU architecture on which this measurement was taken, for example
    /// "aarch64" or "x86_64".
    pub arch: Cow<'a, str>,

    /// The file path of the Wasm benchmark program.
    pub wasm: Cow<'a, str>,

    /// The phase in a Wasm program's lifecycle that was measured: compilation,
    /// instantiation, or execution.
    pub phase: Phase,

    /// The event that was measured: micro seconds of wall time, CPU cycles
    /// executed, instructions retired, cache misses, etc.
    pub event: Cow<'a, str>,

    /// The first engine's results.
    pub a_results: EngineResult,

    /// The second engine's results.
    pub b_results: EngineResult,

    /// The significance level for the confidence interval.
    ///
    /// This is always between 0.0 and 1.0. Typical values are 0.01 and 0.05
    /// which correspond to 99% confidence and 95% confidence respectively.
    pub significance_level: f64,

    /// The half-width confidence interval, i.e. the `i` in
    ///
    /// ```text
    /// b_mean - a_mean ± i
    /// ```
    pub half_width_confidence_interval: f64,
}

impl EffectSize<'_> {
    /// Is the difference between `self.a_mean` and `self.b_mean` statistically
    /// significant?
    pub fn is_significant(&self) -> bool {
        (self.a_results.mean - self.b_results.mean).abs()
            > self.half_width_confidence_interval.abs()
    }

    /// Return `b`'s speedup over `a` and the speedup's confidence interval.
    pub fn b_speed_up_over_a(&self) -> (f64, f64) {
        (
            self.b_results.mean / self.a_results.mean,
            self.half_width_confidence_interval / self.a_results.mean,
        )
    }

    /// Return `a`'s speed up over `b` and the speed up's confidence interval.
    pub fn a_speed_up_over_b(&self) -> (f64, f64) {
        (
            self.a_results.mean / self.b_results.mean,
            self.half_width_confidence_interval / self.b_results.mean,
        )
    }
}
