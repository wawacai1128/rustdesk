use super::*;
use scrap::codec::{Quality, BR_BALANCED};
use std::time::Duration;

// 常量定义
pub const FPS: u32 = 59;          // 默认FPS值
pub const MIN_FPS: u32 = 59;       // 最小FPS值
pub const MAX_FPS: u32 = 120;      // 最大FPS值

// 比特率比例常量
const BR_MAX: f32 = 40.0;
const BR_MIN: f32 = 0.2;
const BR_MIN_HIGH_RESOLUTION: f32 = 0.1;
const MAX_BR_MULTIPLE: f32 = 1.0;

// 用户会话数据结构
#[derive(Default, Debug, Clone)]
struct UserData {
    quality: Option<(i64, Quality)>, // (时间戳, 画质设置)
    record: bool,                    // 是否在录制
}

// 显示数据结构
#[derive(Default, Debug, Clone)]
struct DisplayData {
    support_changing_quality: bool,  // 是否支持改变画质
}

// 视频QoS主控制器
pub struct VideoQoS {
    fps: u32,                       // 当前FPS值
    ratio: f32,                     // 当前比特率比例
    users: HashMap<i32, UserData>,  // 用户会话映射
    displays: HashMap<String, DisplayData>, // 显示设备映射
    bitrate_store: u32,             // 存储的比特率
    fixed_fps: Option<u32>,         // 固定FPS设置
}

impl Default for VideoQoS {
    fn default() -> Self {
        VideoQoS {
            fps: FPS,
            ratio: BR_BALANCED,
            users: Default::default(),
            displays: Default::default(),
            bitrate_store: 0,
            fixed_fps: None,
        }
    }
}

impl VideoQoS {
    // 设置或取消固定FPS
    pub fn set_fixed_fps(&mut self, fps: Option<u32>) {
        if let Some(fps) = fps {
            // 确保FPS在有效范围内
            self.fixed_fps = Some(fps.clamp(MIN_FPS, MAX_FPS));
            self.fps = self.fixed_fps.unwrap(); // 立即应用
        } else {
            self.fixed_fps = None;
            self.fps = FPS; // 回退到默认FPS
        }
    }
    
    // 获取当前固定FPS状态
    pub fn fixed_fps(&self) -> Option<u32> {
        self.fixed_fps
    }

    // 计算每帧时间
    pub fn spf(&self) -> Duration {
        Duration::from_secs_f32(1.0 / (self.fps() as f32))
    }

    // 获取当前FPS
    pub fn fps(&self) -> u32 {
        // 优先使用固定FPS
        if let Some(fixed_fps) = self.fixed_fps {
            return fixed_fps;
        }
        self.fps
    }

    // 存储比特率
    pub fn store_bitrate(&mut self, bitrate: u32) {
        self.bitrate_store = bitrate;
    }

    // 获取比特率
    pub fn bitrate(&self) -> u32 {
        self.bitrate_store
    }

    // 获取比特率比例
    pub fn ratio(&mut self) -> f32 {
        if self.ratio < BR_MIN_HIGH_RESOLUTION || self.ratio > BR_MAX {
            self.ratio = BR_BALANCED;
        }
        self.ratio
    }

    // 检查是否有用户正在录制
    pub fn record(&self) -> bool {
        self.users.iter().any(|u| u.1.record)
    }

    // 设置是否支持改变画质
    pub fn set_support_changing_quality(&mut self, video_service_name: &str, support: bool) {
        if let Some(display) = self.displays.get_mut(video_service_name) {
            display.support_changing_quality = support;
        }
    }

    // 检查是否启用VBR
    pub fn in_vbr_state(&self) -> bool {
        // 简化的VBR状态检查
        self.displays.iter().all(|e| e.1.support_changing_quality)
    }
}

// 用户会话管理
impl VideoQoS {
    // 初始化新用户会话
    pub fn on_connection_open(&mut self, id: i32) {
        self.users.insert(id, UserData::default());
    }

    // 清理用户会话
    pub fn on_connection_close(&mut self, id: i32) {
        self.users.remove(&id);
        if self.users.is_empty() {
            *self = Default::default();
        }
    }

    // 用户设置画质
    pub fn user_image_quality(&mut self, id: i32, image_quality: i32) {
        let convert_quality = |q: i32| -> Quality {
            match q {
                _ if q == ImageQuality::Balanced.value() => Quality::Balanced,
                _ if q == ImageQuality::Low.value() => Quality::Low,
                _ if q == ImageQuality::Best.value() => Quality::Best,
                _ => {
                    let b = ((q >> 8 & 0xFFF) * 2) as f32 / 100.0;
                    Quality::Custom(b.clamp(BR_MIN, BR_MAX))
                }
            }
        };

        let quality = Some((hbb_common::get_time(), convert_quality(image_quality)));
        if let Some(user) = self.users.get_mut(&id) {
            user.quality = quality;
            // 直接更新比例
            self.ratio = self.latest_quality().ratio();
        }
    }

    // 用户录制状态
    pub fn user_record(&mut self, id: i32, v: bool) {
        if let Some(user) = self.users.get_mut(&id) {
            user.record = v;
        }
    }
}

// 显示管理
impl VideoQoS {
    // 添加新显示
    pub fn new_display(&mut self, video_service_name: String) {
        self.displays.insert(
            video_service_name, 
            DisplayData {
                support_changing_quality: true, // 默认支持
            }
        );
    }

    // 移除显示
    pub fn remove_display(&mut self, video_service_name: &str) {
        self.displays.remove(video_service_name);
    }

    // 更新显示数据 (简化版本)
    pub fn update_display_data(&mut self, _video_service_name: &str, _send_counter: usize) {
        // 在固定FPS模式下不需要特殊处理
        // 保留函数签名以保持兼容性
    }

    // 获取最新画质设置
    pub fn latest_quality(&self) -> Quality {
        self.users
            .iter()
            .filter_map(|(_, u)| u.quality)
            .max_by_key(|(timestamp, _)| *timestamp)
            .map(|(_, quality)| quality)
            .unwrap_or(Quality::Balanced)
    }
}
