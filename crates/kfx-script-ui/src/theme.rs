//! Colours of the editor. The host passes its own [`Theme`]; the default is dark with warm gold
//! accents.
use crate::lang::HlClass;
use egui::Color32;

#[derive(Clone, Debug)]
pub struct Theme {
    pub background: Color32,
    pub panel: Color32,
    pub gutter_bg: Color32,
    pub gutter_fg: Color32,
    pub gutter_fg_active: Color32,
    pub current_line: Color32,
    pub selection: Color32,
    pub text: Color32,
    pub command: Color32,
    pub keyword: Color32,
    pub function: Color32,
    pub number: Color32,
    pub string: Color32,
    pub constant: Color32,
    pub operator: Color32,
    pub punctuation: Color32,
    pub comment: Color32,
    pub error_token: Color32,
    pub diag_error: Color32,
    pub diag_warning: Color32,
    pub diag_info: Color32,
    pub block_match: Color32,
    pub find_match: Color32,
    pub find_current: Color32,
    /// Accent for buttons, active items and meters.
    pub accent: Color32,
    pub meter_ok: Color32,
    pub meter_near: Color32,
    pub meter_over: Color32,
    /// Lua annotations (`---@param`) and the types inside them.
    pub annotation: Color32,
    pub annotation_type: Color32,
    pub local: Color32,
    pub parameter: Color32,
    pub api_function: Color32,
    pub api_global: Color32,
    pub std_lib: Color32,
    pub user_global: Color32,
    pub unknown: Color32,
    pub field: Color32,
    pub font_size: f32,
    /// Dark base visuals (false: the light egui visuals are used for buttons and panels).
    pub is_dark: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::dark()
    }
}

impl Theme {
    pub fn dark() -> Theme {
        let c = Color32::from_rgb;
        Theme {
            background: c(28, 26, 23),
            panel: c(36, 33, 29),
            gutter_bg: c(33, 30, 27),
            gutter_fg: c(112, 104, 92),
            gutter_fg_active: c(232, 190, 110),
            current_line: Color32::from_rgba_unmultiplied(255, 220, 150, 14),
            selection: Color32::from_rgba_unmultiplied(214, 160, 60, 80),
            text: c(216, 207, 192),
            command: c(120, 178, 222),
            keyword: c(236, 172, 58),
            function: c(196, 156, 232),
            number: c(228, 146, 108),
            string: c(196, 180, 112),
            constant: c(158, 204, 136),
            operator: c(206, 200, 190),
            punctuation: c(150, 143, 132),
            comment: c(112, 124, 104),
            error_token: c(244, 100, 88),
            diag_error: c(240, 84, 76),
            diag_warning: c(240, 180, 60),
            diag_info: c(100, 170, 230),
            block_match: Color32::from_rgba_unmultiplied(255, 200, 80, 70),
            find_match: Color32::from_rgba_unmultiplied(120, 170, 240, 70),
            find_current: Color32::from_rgba_unmultiplied(255, 170, 40, 130),
            accent: c(224, 169, 58),
            meter_ok: c(120, 170, 100),
            meter_near: c(240, 170, 40),
            meter_over: c(232, 70, 62),
            annotation: c(122, 160, 120),
            annotation_type: c(104, 170, 160),
            local: c(214, 206, 190),
            parameter: c(226, 190, 150),
            api_function: c(120, 178, 222),
            api_global: c(158, 204, 136),
            std_lib: c(150, 170, 200),
            user_global: c(222, 186, 120),
            unknown: c(180, 150, 140),
            field: c(190, 200, 214),
            font_size: 14.0,
            is_dark: true,
        }
    }

    pub fn light() -> Theme {
        let c = Color32::from_rgb;
        Theme {
            background: c(252, 250, 245),
            panel: c(244, 240, 232),
            gutter_bg: c(240, 236, 228),
            gutter_fg: c(150, 140, 125),
            gutter_fg_active: c(150, 100, 20),
            current_line: Color32::from_rgba_unmultiplied(200, 150, 40, 24),
            selection: Color32::from_rgba_unmultiplied(214, 160, 60, 90),
            text: c(50, 44, 36),
            command: c(20, 100, 170),
            keyword: c(176, 104, 0),
            function: c(120, 70, 170),
            number: c(180, 80, 40),
            string: c(130, 110, 20),
            constant: c(50, 120, 50),
            operator: c(70, 66, 60),
            punctuation: c(120, 112, 100),
            comment: c(120, 130, 110),
            error_token: c(200, 40, 30),
            diag_error: c(210, 40, 36),
            diag_warning: c(200, 130, 0),
            diag_info: c(30, 110, 190),
            block_match: Color32::from_rgba_unmultiplied(230, 160, 20, 80),
            find_match: Color32::from_rgba_unmultiplied(60, 120, 220, 60),
            find_current: Color32::from_rgba_unmultiplied(255, 150, 0, 120),
            accent: c(176, 120, 10),
            meter_ok: c(70, 140, 70),
            meter_near: c(210, 140, 0),
            meter_over: c(200, 40, 36),
            annotation: c(70, 120, 70),
            annotation_type: c(30, 120, 110),
            local: c(60, 54, 46),
            parameter: c(150, 90, 20),
            api_function: c(20, 100, 170),
            api_global: c(50, 120, 50),
            std_lib: c(60, 90, 140),
            user_global: c(140, 90, 0),
            unknown: c(150, 90, 80),
            field: c(60, 80, 110),
            font_size: 14.0,
            is_dark: false,
        }
    }

    pub fn color_for(&self, c: HlClass) -> Color32 {
        match c {
            HlClass::Command => self.command,
            HlClass::Keyword => self.keyword,
            HlClass::Function => self.function,
            HlClass::Number => self.number,
            HlClass::String => self.string,
            HlClass::Constant => self.constant,
            HlClass::Operator => self.operator,
            HlClass::Punctuation => self.punctuation,
            HlClass::Comment => self.comment,
            HlClass::Error => self.error_token,
            HlClass::Annotation => self.annotation,
            HlClass::AnnotationType => self.annotation_type,
            HlClass::Local => self.local,
            HlClass::Parameter => self.parameter,
            HlClass::ApiFunction => self.api_function,
            HlClass::ApiGlobal => self.api_global,
            HlClass::Std => self.std_lib,
            HlClass::UserGlobal => self.user_global,
            HlClass::Unknown => self.unknown,
            HlClass::Field => self.field,
            HlClass::Method => self.api_function,
        }
    }

    /// Colour of a tag or status item.
    pub fn tone_color(&self, t: crate::lang::Tone) -> Color32 {
        use crate::lang::Tone;
        match t {
            Tone::Plain => self.text,
            Tone::Keyword => self.keyword,
            Tone::Function => self.function,
            Tone::Constant => self.constant,
            Tone::Good => self.meter_ok,
            Tone::Bad => self.diag_error,
            Tone::Muted => self.punctuation,
        }
    }

    /// The same theme with another font size.
    pub fn with_font_size(mut self, size: f32) -> Theme {
        self.font_size = size.clamp(8.0, 32.0);
        self
    }

    pub fn severity_color(&self, s: kfx_script::Severity) -> Color32 {
        match s {
            kfx_script::Severity::Error => self.diag_error,
            kfx_script::Severity::Warning => self.diag_warning,
            kfx_script::Severity::Info => self.diag_info,
        }
    }
}
