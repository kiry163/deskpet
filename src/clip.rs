//! 动画解码：libvpx 解码 VP9 主色 + alpha，合成 BGRA（premultiplied）。
#![allow(dead_code)]
use std::rc::Rc;

use crate::vpx::{vpx_image_t, VPX_PLANE_ALPHA, VPX_PLANE_U, VPX_PLANE_V, VPX_PLANE_Y};
use crate::webm::WebM;

pub const W: usize = 640;
pub const H: usize = 360;

/// 单个动画的逐帧解码器。
///
/// 解码器不在此持有：全部动画共享 Pet 级的一组解码器（一次只播一个动画，
/// 且各动画首帧均为 VP9 关键帧——见 check_keyframe，解码关键帧即重置参考帧
/// 状态）。若每动画独立持有一组解码器，其惰性分配的帧缓冲池会累积
/// ~6MB×动画数（51 段 ≈ 300MB）且不复用，长时间运行内存持续增长。
pub struct ClipDecoder {
    pub webm: Rc<WebM>,
    pub cur: usize,
    // 查找表
    ytab: [i32; 256],
}

impl ClipDecoder {
    pub fn new(webm: Rc<WebM>) -> Option<ClipDecoder> {
        // 预计算 YUV->RGB 有限范围 BT.601 亮度系数
        let mut ytab = [0i32; 256];
        for i in 0..256usize {
            let y = (i as i32 - 16).max(0);
            ytab[i] = (298 * y) >> 8;
        }
        Some(ClipDecoder {
            webm,
            cur: 0,
            ytab,
        })
    }

    /// 跳到指定帧（重播时从 0 开始）。
    ///
    /// 无需重建/重置解码器：全部素材首帧均为 VP9 关键帧（check_keyframe 已验证
    /// 51/51，含 alpha 帧），解码关键帧即重置参考帧状态。解码器由 Pet 级共享，
    /// 常驻整个生命周期（见结构体注释）。
    pub fn seek(&mut self, idx: usize) {
        if self.cur == idx {
            return;
        }
        self.cur = idx;
    }

    pub fn frame_count(&self) -> usize {
        self.webm.frames.len()
    }

    pub fn duration_ms(&self) -> u64 {
        self.webm.duration_ms()
    }

    /// 当前帧时间（秒）。
    pub fn current_time_secs(&self) -> f64 {
        if self.cur == 0 || self.webm.frames.is_empty() {
            return 0.0;
        }
        let idx = (self.cur - 1).min(self.webm.frames.len() - 1);
        self.webm.frames[idx].timecode_ms as f64 / 1000.0
    }

    /// 解码并返回当前帧 BGRA（premultiplied alpha），推进到下一帧。
    /// 返回借用的合成缓冲（Pet 级共享，复用不重新分配）；None 表示播完。
    /// `color_dec` / `alpha_dec` / `out` 均为 Pet 级共享资源（本 clip 不持有）。
    pub fn next_frame<'o>(
        &mut self,
        color_dec: &mut crate::vpx::Decoder,
        alpha_dec: Option<&mut crate::vpx::Decoder>,
        out: &'o mut Vec<u8>,
    ) -> Option<&'o [u8]> {
        if self.cur >= self.webm.frames.len() {
            return None;
        }
        // 借用帧数据（不 clone，避免每帧 ~921KB 拷贝 + 分配）
        let f = &self.webm.frames[self.cur];
        self.cur += 1;
        let color = color_dec.decode(&f.video)?;
        let alpha = match (&f.alpha, alpha_dec) {
            (Some(a), Some(ad)) => ad.decode(a),
            _ => None,
        };
        compose_into(color, alpha, &self.ytab, out);
        Some(out)
    }
}

/// 合成 I420 主色 + 可选 alpha（灰度 I420 的 Y 平面）→ BGRA premultiplied，
/// 写入复用缓冲 out（避免每帧分配 ~921KB）。
fn compose_into(color: &vpx_image_t, alpha: Option<&vpx_image_t>, ytab: &[i32; 256], out: &mut Vec<u8>) {
    // libvpx 输出的 w/h 是存储对齐尺寸，d_w/d_h 才是实际像素尺寸
    let w = color.d_w as usize;
    let h = color.d_h as usize;
    out.resize(w * h * 4, 0);
    let out = out.as_mut_slice();

    let yp = color.planes[VPX_PLANE_Y];
    let up = color.planes[VPX_PLANE_U];
    let vp = color.planes[VPX_PLANE_V];
    let ys = color.stride[VPX_PLANE_Y] as usize;
    let us = color.stride[VPX_PLANE_U] as usize;
    let vs = color.stride[VPX_PLANE_V] as usize;
    let csx = color.x_chroma_shift as usize;
    let csy = color.y_chroma_shift as usize;

    let (ap, as_): (*const u8, usize) = match alpha {
        Some(a) => {
            if a.planes[VPX_PLANE_ALPHA].is_null() {
                (a.planes[VPX_PLANE_Y], a.stride[VPX_PLANE_Y] as usize)
            } else {
                (a.planes[VPX_PLANE_ALPHA], a.stride[VPX_PLANE_ALPHA] as usize)
            }
        }
        None => (std::ptr::null(), 0),
    };
    let has_alpha = alpha.is_some();

    for y in 0..h {
        let row_off = y * w * 4;
        let y_row = y * ys;
        let u_row = (y >> csy) * us;
        let v_row = (y >> csy) * vs;
        let a_row = if has_alpha { y * as_ } else { 0 };
        for x in 0..w {
            let yv = unsafe { *yp.add(y_row + x) } as usize;
            let u = unsafe { *up.add(u_row + (x >> csx)) } as usize;
            let v = unsafe { *vp.add(v_row + (x >> csx)) } as usize;
            let base = ytab[yv];
            let r = (base + ((409 * (v as i32 - 128) + 128) >> 8)).clamp(0, 255) as u8;
            let g = (base - ((100 * (u as i32 - 128) + 128) >> 8) - ((208 * (v as i32 - 128) + 128) >> 8)).clamp(0, 255) as u8;
            let b = (base + ((516 * (u as i32 - 128) + 128) >> 8)).clamp(0, 255) as u8;
            let mut a = 255u8;
            if has_alpha {
                a = unsafe { *ap.add(a_row + x) };
            }
            // premultiplied alpha
            let o = row_off + x * 4;
            out[o] = ((b as u32 * a as u32) >> 8) as u8;
            out[o + 1] = ((g as u32 * a as u32) >> 8) as u8;
            out[o + 2] = ((r as u32 * a as u32) >> 8) as u8;
            out[o + 3] = a;
        }
    }
}
