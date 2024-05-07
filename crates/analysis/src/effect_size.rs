use crate::keys::KeyBuilder;
use anyhow::Result;
use sightglass_data::{EffectSize, EngineResult, Measurement, Phase, Summary};
use std::{collections::BTreeSet, io::Write};

/// Find the effect size (and confidence interval) of between two different
/// engines (i.e. two different commits of Wasmtime).
///
/// This allows us to justify statements like "we are 99% confident that the new
/// register allocator is 13.6% faster (± 1.7%) than the old register
/// allocator."
///
/// This can only test differences between the results for exactly two different
/// engines. If there aren't exactly two different engines represented in
/// `measurements` then an error is returned.
pub fn calculate<'a>(
    significance_level: f64,
    measurements: &[Measurement<'a>],
) -> Result<Vec<EffectSize<'a>>> {
    anyhow::ensure!(
        0.0 <= significance_level && significance_level <= 1.0,
        "The significance_level must be between 0.0 and 1.0. \
             Typical values are 0.05 and 0.01 (i.e. 95% and 99% confidence). \
             Found {}.",
        significance_level,
    );

    let keys = KeyBuilder::all().engine(false).keys(measurements);
    let mut results = Vec::with_capacity(keys.len());

    for key in keys {
        let key_measurements: Vec<_> = measurements.iter().filter(|m| key.matches(m)).collect();

        // NB: `BTreeSet` so they're always sorted.
        let engines: BTreeSet<_> = key_measurements
            .iter()
            .map(|m| m.engine_and_flags())
            .collect();
        anyhow::ensure!(
            engines.len() == 2,
            "Can only test significance between exactly two different engines. Found {} \
                 different engines.",
            engines.len()
        );

        let mut engines = engines.into_iter();
        let engine_a = engines.next().unwrap();
        let engine_b = engines.next().unwrap();

        let a: behrens_fisher::Stats = key_measurements
            .iter()
            .filter(|m| m.engine_and_flags() == engine_a)
            .map(|m| m.count as f64)
            .collect();
        let b: behrens_fisher::Stats = key_measurements
            .iter()
            .filter(|m| m.engine_and_flags() == engine_b)
            .map(|m| m.count as f64)
            .collect();

        let ci = behrens_fisher::confidence_interval(1.0 - significance_level, a, b)?;
        results.push(EffectSize {
            arch: key.arch.unwrap(),
            wasm: key.wasm.unwrap(),
            phase: key.phase.unwrap(),
            event: key.event.unwrap(),
            a_results: EngineResult {
                engine: engine_a.0.to_string(),
                engine_flags: engine_a.1.to_string(),
                mean: a.mean,
            },
            b_results: EngineResult {
                engine: engine_b.0.to_string(),
                engine_flags: engine_b.1.to_string(),
                mean: b.mean,
            },
            significance_level,
            half_width_confidence_interval: ci,
        });
    }

    Ok(results)
}

/// Write a vector of [EffectSize] structures to the passed `output_file` in human-readable form.
/// The `summaries` are needed
pub fn write(
    mut effect_sizes: Vec<EffectSize<'_>>,
    summaries: &[Summary<'_>],
    significance_level: f64,
    output_file: &mut dyn Write,
) -> Result<()> {
    // Sort the effect sizes so that we focus on statistically significant results before
    // insignificant results and larger relative effect sizes before smaller relative effect sizes.
    effect_sizes.sort_by(|x, y| {
        y.is_significant().cmp(&x.is_significant()).then_with(|| {
            let x_speedup = x.a_speed_up_over_b().0.max(x.b_speed_up_over_a().0);
            let y_speedup = y.a_speed_up_over_b().0.max(y.b_speed_up_over_a().0);
            y_speedup.partial_cmp(&x_speedup).unwrap()
        })
    });

    for effect_size in effect_sizes {
        writeln!(output_file)?;
        writeln!(
            output_file,
            "{} :: {} :: {}",
            effect_size.phase, effect_size.event, effect_size.wasm
        )?;
        writeln!(output_file)?;

        // For readability, trim the shared prefix from our two engine names.
        let end_of_shared_prefix = effect_size
            .a_results
            .engine
            .char_indices()
            .zip(effect_size.b_results.engine.char_indices())
            .find_map(|((i, a), (j, b))| {
                if a == b {
                    None
                } else {
                    debug_assert_eq!(i, j);
                    Some(i)
                }
            })
            .unwrap_or(0);
        let a_engine = &effect_size.a_results.engine[end_of_shared_prefix..];
        let b_engine = &effect_size.b_results.engine[end_of_shared_prefix..];

        if effect_size.is_significant() {
            let mut fast_results = &effect_size.a_results;
            let mut slow_results = &effect_size.b_results;
            let mut fast_engine = a_engine;
            let mut slow_engine = b_engine;
            if fast_results.mean > slow_results.mean {
                std::mem::swap(&mut fast_results, &mut slow_results);
                std::mem::swap(&mut fast_engine, &mut slow_engine);
            }

            writeln!(
                output_file,
                "  Δ = {:.2} ± {:.2} (confidence = {}%)",
                slow_results.mean - fast_results.mean,
                effect_size.half_width_confidence_interval.abs(),
                (1.0 - significance_level) * 100.0,
            )?;
            writeln!(output_file)?;

            let fast_space = if !fast_engine.is_empty() && !fast_results.engine_flags.is_empty() {
                " "
            } else {
                ""
            };
            let slow_space = if !slow_engine.is_empty() && !slow_results.engine_flags.is_empty() {
                " "
            } else {
                ""
            };

            let ratio = slow_results.mean / fast_results.mean;
            let ratio_ci = effect_size.half_width_confidence_interval / fast_results.mean;
            writeln!(
                output_file,
                "  {fast_engine}{fast_space}{fast_flags} is {ratio_min:.2}x to {ratio_max:.2}x faster than {slow_engine}{slow_space}{slow_flags}!",
                fast_flags = fast_results.engine_flags,
                slow_flags = slow_results.engine_flags,
                ratio_min = ratio - ratio_ci,
                ratio_max = ratio + ratio_ci,
            )?;
        } else {
            writeln!(output_file, "  No difference in performance.")?;
        }
        writeln!(output_file)?;

        let get_summary =
            |engine: &str, engine_flags: &str, wasm: &str, phase: Phase, event: &str| {
                // TODO this sorting is not using `arch` which is not guaranteed to be the same in
                // result sets; potentially this could re-use `Key` functionality.
                summaries
                    .iter()
                    .find(|s| {
                        s.engine == engine
                            && s.engine_flags == engine_flags
                            && s.wasm == wasm
                            && s.phase == phase
                            && s.event == event
                    })
                    .unwrap()
            };

        let a_summary = get_summary(
            &effect_size.a_results.engine,
            &effect_size.a_results.engine_flags,
            &effect_size.wasm,
            effect_size.phase,
            &effect_size.event,
        );
        writeln!(
            output_file,
            "  [{} {:.2} {}] {}",
            a_summary.min, a_summary.mean, a_summary.max, a_engine,
        )?;

        let b_summary = get_summary(
            &effect_size.b_results.engine,
            &effect_size.b_results.engine_flags,
            &effect_size.wasm,
            effect_size.phase,
            &effect_size.event,
        );
        writeln!(
            output_file,
            "  [{} {:.2} {}] {}",
            b_summary.min, b_summary.mean, b_summary.max, b_engine,
        )?;
    }

    Ok(())
}
