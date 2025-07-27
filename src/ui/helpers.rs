use std::collections::HashMap;

use ratatui::layout::Rect;

use crate::{app::CounterMode, services};

pub(crate) fn truncate_fit(s: &str, max_width: usize) -> String {
    if s.len() > max_width {
        let mut truncated = s[..(max_width.saturating_sub(1))].to_string();
        truncated.push('…');
        truncated
    } else {
        s.to_string()
    }
}

/// Calculate column widths based on ratios.
pub(crate) fn compute_column_widths(total_width: u16, ratios: &[f64]) -> Vec<usize> {
    let total = total_width as f64;
    let mut widths = Vec::with_capacity(ratios.len());
    for &ratio in ratios {
        let col_width = (ratio * total).round().max(2.0) as usize;
        widths.push(col_width);
    }
    widths
}

pub(crate) fn centered_rect_percent_w_lines_h(percent_x: u16, y_height: u16, r: Rect) -> (u16, u16, u16, u16) {
    let popup_width = r.width * percent_x / 100;
    let popup_height = y_height;
    let x = r.x + (r.width.saturating_sub(popup_width)) / 2;
    let y = r.y + (r.height.saturating_sub(popup_height)) / 2;
    (x, y, popup_width, popup_height)
}

pub(crate) fn centered_rect_percent(percent_x: u16, percent_y: u16, r: Rect) -> (u16, u16, u16, u16) {
    let popup_width = r.width * percent_x / 100;
    let popup_height = r.height * percent_y / 100;
    let x = r.x + (r.width.saturating_sub(popup_width)) / 2;
    let y = r.y + (r.height.saturating_sub(popup_height)) / 2;
    (x, y, popup_width, popup_height)
}


/// Compute receive/send bandwidth in Gbps based on a performance counter.
pub(crate) fn get_bw(
    perfcounters: &HashMap<String, u64>,
    counter: &str,
    counter_mode: &CounterMode,
) -> f64 {
    let mut time_delta = *perfcounters.get("end_timestamp").unwrap_or(&1) as f64;
    if let CounterMode::Delta = counter_mode {
        time_delta /= 1e9;
    } else {
        time_delta = 1.0;
    }

    perfcounters
        .get(counter)
        .map(|&val| (val as f64 * 4.0 / 1e9 * 8.0) / time_delta)
        .unwrap_or(0.0)
}

/// Compute bandwidth loss in Gbps based on a performance counter.
pub(crate) fn get_bw_loss(
    perfcounters: &HashMap<String, u64>,
    counter: &str,
    counter_mode: &CounterMode,
) -> f64 {
    let mut time_delta = *perfcounters.get("end_timestamp").unwrap_or(&1) as f64;
    if let CounterMode::Delta = counter_mode {
        time_delta /= 1e9;
    } else {
        time_delta = 1.0;
    }

    perfcounters
        .get(counter)
        .map(|&val| val as f64 * 64.0 / 1e9 / time_delta)
        .unwrap_or(0.0)
}

/// Count all error counters and return the sum.
pub(crate) fn count_errors(perfcounters: &HashMap<String, u64>) -> u128 {
    services::rsmad::ERROR_COUNTERS
        .iter()
        .filter_map(|&err_ctr| perfcounters.get(err_ctr))
        .map(|&val| val as u128)
        .sum()
}

/// Get a comma separated string of error counter names with non-zero values.
pub(crate) fn get_error_strings(perfcounters: &HashMap<String, u64>) -> String {
    let errors: Vec<String> = services::rsmad::ERROR_COUNTERS
        .iter()
        .filter_map(|&err_ctr| perfcounters.get_key_value(err_ctr))
        .filter(|e| *e.1 > 0)
        .map(|e| e.0.to_string())
        .collect();

    errors.join(",")
}

