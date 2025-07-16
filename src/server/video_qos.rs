use super::*;
use scrap::codec::{Quality, BR_BALANCED, BR_BEST, BR_SPEED};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

// Constants - 只保留固定FPS相关常量
pub const FPS: u32 = 60;
pub const MIN_FPS: u32 = 60;
pub const MAX_FPS: u32 = 120;
pub const INIT_FPS: u32 = 60;

// Bitrate ratio constants
const BR_MAX: f32 = 40.0;
const BR_MIN: f32 = 0.2;
const BR_MIN_HIGH_RESOLUTION: f32 = 0.1;
const MAX_BR_MULTIPLE: f32 = 1.0;

const ADJUST_RATIO_INTERVAL: usize = 3;
const DYNAMIC_SCREEN_THRESHOLD: usize = 2;
const DELAY_THRESHOLD_150MS: u32 = 150;

// 简化用户数据结构 - 删除所有延迟相关字段
#[derive(Default, Debug, Clone)]
struct UserData {
    custom_fps: Option<u32>,
    quality: Option<(i64, Quality)>,
    record: bool,
}

#[derive(Default, Debug, Clone)]
struct DisplayData {
    send_counter: usize,
    support_changing_quality: bool,
}

// 主QoS控制器
pub struct VideoQoS {
    fps: u32,  // 当前FPS值
    ratio: f32,
    users: HashMap<i32, UserData>,
    displays: HashMap<String, DisplayData>,
    bitrate_store: u32,
    adjust_ratio_instant: Instant,
    abr_config: bool,
    fixed_fps: Option<u32>, // 固定FPS配置
}

impl Default for VideoQoS {
    fn default() -> Self {
        VideoQoS {
            fps: FPS,
            ratio: BR_BALANCED,
            users: Default::default(),
            displays: Default::default(),
            bitrate_store: 0,
            adjust_ratio_instant: Instant::now(),
            abr_config: true,
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
        Duration::from_secs_f32(1. / (self.fps() as f32))
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
        self.abr_config && self.displays.iter().all(|e| e.1.support_changing_quality)
    }
}

// 用户会话管理
impl VideoQoS {
    // 初始化新用户会话
    pub fn on_connection_open(&mut self, id: i32) {
        self.users.insert(id, UserData::default());
        self.abr_config = Config::get_option("enable-abr") != "N";
    }

    // 清理用户会话
    pub fn on_connection_close(&mut self, id: i32) {
        self.users.remove(&id);
        if self.users.is_empty() {
            *self = Default::default();
        }
    }

    // 用户自定义FPS (保留但不再使用)
    pub fn user_custom_fps(&mut self, id: i32, fps: u32) {
        // 仅记录，不影响实际FPS
        if fps < MIN_FPS || fps > MAX_FPS {
            return;
        }
        if let Some(user) = self.users.get_mut(&id) {
            user.custom_fps = Some(fps);
        }
    }

    // 用户设置画质
    pub fn user_image_quality(&mut self, id: i32, image_quality: i32) {
        let convert_quality = |q: i32| -> Quality {
            if q == ImageQuality::Balanced.value() {
                Quality::Balanced
            } else if q == ImageQuality::Low.value() {
                Quality::Low
            } else if q == ImageQuality::Best.value() {
                Quality::Best
            } else {
                let b = ((q >> 8 & 0xFFF) * 2) as f32 / 100.0;
                Quality::Custom(b.clamp(BR_MIN, BR_MAX))
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
        self.displays
            .insert(video_service_name, DisplayData::default());
    }

    // 移除显示
    pub fn remove_display(&mut self, video_service_name: &str) {
        self.displays.remove(video_service_name);
    }

    // 更新显示数据 (只处理画质调整)
    pub fn update_display_data(&mut self, video_service_name: &str, send_counter: usize) {
        if let Some(display) = self.displays.get_mut(video_service_name) {
            display.send_counter += send_counter;
        }
        
        let abr_enabled = self.in_vbr_state();
        if abr_enabled {
            if self.adjust_ratio_instant.elapsed().as_secs() >= ADJUST_RATIO_INTERVAL as u64 {
                let dynamic_screen = self
                    .displays
                    .iter()
                    .any(|d| d.1.send_counter >= ADJUST_RATIO_INTERVAL * DYNAMIC_SCREEN_THRESHOLD);
                
                self.displays.iter_mut().for_each(|d| {
                    d.1.send_counter = 0;
                });
                
                self.adjust_ratio(dynamic_screen);
            }
        } else {
            self.ratio = self.latest_quality().ratio();
        }
    }

    // 获取最新画质设置
    pub fn latest_quality(&self) -> Quality {
        self.users
            .iter()
            .map(|(_, u)| u.quality)
            .filter(|q| *q != None)
            .max_by(|a, b| a.unwrap_or_default().0.cmp(&b.unwrap_or_default().0))
            .flatten()
            .unwrap_or((0, Quality::Balanced))
            .1
    }

    // 调整画质比例
    fn adjust_ratio(&mut self, dynamic_screen: bool) {
        if !self.in_vbr_state() {
            return;
        }
        
        let target_quality = self.latest_quality();
        let target_ratio = self.latest_quality().ratio();
        let current_ratio = self.ratio;
        let current_bitrate = self.bitrate();

        // 计算高分辨率最小比例
        let ratio_1mbps = if current_bitrate > 0 {
            Some((current_ratio * 1000.0 / current_bitrate as f32).max(BR_MIN_HIGH_RESOLUTION))
        } else {
            None
        };

        // 计算增加150kbps的比例
        let ratio_add_150kbps = if current_bitrate > 0 {
            Some((current_bitrate + 150) as f32 * current_ratio / current_bitrate as f32)
        } else {
            None
        };

        // 设置基于画质模式的最小比例
        let min = match target_quality {
            Quality::Best => {
                let mut min = BR_BEST / 2.5;
                if let Some(ratio_1mbps) = ratio_1mbps {
                    if min > ratio_1mbps {
                        min = ratio_1mbps;
                    }
                }
                min.max(BR_MIN)
            }
            Quality::Balanced => {
                let mut min = (BR_BALANCED / 2.0).min(0.4);
                if let Some(ratio_1mbps) = ratio_1mbps {
                    if min > ratio_1mbps {
                        min = ratio_1mbps;
                    }
                }
                min.max(BR_MIN_HIGH_RESOLUTION)
            }
            Quality::Low => BR_MIN_HIGH_RESOLUTION,
            Quality::Custom(_) => BR_MIN_HIGH_RESOLUTION,
        };
        
        let max = target_ratio * MAX_BR_MULTIPLE;
        let mut v = current_ratio;

        // 根据动态屏幕调整比例
        if dynamic_screen {
            v = current_ratio * 1.15;
        }

        // 限制质量增加率
        if let Some(ratio_add_150kbps) = ratio_add_150kbps {
            if v > ratio_add_150kbps
                && ratio_add_150kbps > current_ratio
                && current_ratio >= BR_SPEED
            {
                v = ratio_add_150kbps;
            }
        }

        self.ratio = v.clamp(min, max);
        self.adjust_ratio_instant = Instant::now();
    }
}
