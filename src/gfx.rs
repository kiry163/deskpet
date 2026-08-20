//! 平台无关的像素处理：BGRA 缩放 + 水平镜像。

/// 把 src（sw×sh BGRA）缩放/镜像到 dst（dw×dh，行跨距 dst_stride）。
pub fn scale_bgra(
    src: &[u8],
    sw: usize,
    sh: usize,
    dst: &mut [u8],
    dw: usize,
    dh: usize,
    dst_stride: usize,
    mirror: bool,
) {
    for y in 0..dh {
        let sy = (y as u64 * sh as u64 / dh as u64) as usize;
        let srow = sy * sw * 4;
        let drow = y * dst_stride;
        for x in 0..dw {
            let sx = if mirror {
                sw - 1 - (x as u64 * sw as u64 / dw as u64) as usize
            } else {
                (x as u64 * sw as u64 / dw as u64) as usize
            };
            let s = srow + sx * 4;
            let d = drow + x * 4;
            dst[d] = src[s];
            dst[d + 1] = src[s + 1];
            dst[d + 2] = src[s + 2];
            dst[d + 3] = src[s + 3];
        }
    }
}
