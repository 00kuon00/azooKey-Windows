use tao::dpi::{LogicalSize, Size};
use tao::window::Window;
use windows::Win32::{
    Foundation::RECT,
    Graphics::Gdi::{GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST},
};

pub fn get_candidate_window_position(
    top: i32,
    left: i32,
    bottom: i32,
    right: i32,
    window: &Window,
) -> (f64, f64) {
    let mut x = left - 15;
    let mut y = bottom;

    let monitor = unsafe {
        MonitorFromRect(
            &RECT {
                left,
                top,
                right,
                bottom,
            } as *const _,
            MONITOR_DEFAULTTONEAREST,
        )
    };

    let mut monitor_info = MONITORINFO::default();
    monitor_info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;

    unsafe {
        let _ = GetMonitorInfoW(monitor, &mut monitor_info);
    }

    // If the bottom of the candidate window is hidden, show it above
    y = if y + window.inner_size().height as i32 > monitor_info.rcWork.bottom {
        top - window.inner_size().height as i32
    } else {
        y
    };

    // If the right of the candidate window is hidden, show it to the left
    x = if x + window.inner_size().width as i32 > monitor_info.rcWork.right {
        monitor_info.rcWork.right - window.inner_size().width as i32
    } else {
        x
    };

    // If the left of the candidate window is hidden, show it to the right
    x = if x < monitor_info.rcWork.left {
        monitor_info.rcWork.left
    } else {
        x
    };

    (x as f64, y as f64)
}

/// 候補ウィンドウの幅（候補の最大文字数から決める）。
/// 論理ピクセルで返す。物理ピクセルで決めると、表示倍率が高い画面で幅だけ狭くなる（225% で 100 相当）
pub fn candidate_window_width(max_len: u32) -> Size {
    Size::Logical(LogicalSize::new(
        std::cmp::max(225, 120 + max_len * 18) as f64,
        0.0,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 表示倍率 225% でも、100% のときと同じ見た目の幅になる
    #[test]
    fn candidate_width_scales_with_display() {
        let width = |max_len, scale| {
            candidate_window_width(max_len)
                .to_physical::<u32>(scale)
                .width
        };
        assert_eq!(width(0, 1.0), 225);
        assert_eq!(width(0, 2.25), 506);
        assert_eq!(width(10, 2.25), 675);
    }
}
