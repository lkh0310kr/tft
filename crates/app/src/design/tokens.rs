//! The colours and metrics the shell paints with, fixed for the dark
//! and light base themes. Surface and semantic values follow the measured
//! theme; TFT's primary interactive accent is `#0083f0`.

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Color {
    pub h: f32,
    pub s: f32,
    pub l: f32,
    pub a: f32,
}

use Color as Hsla;

/// TFT's primary interactive color (`#0083f0`).
const PRIMARY_ACCENT: Hsla = Hsla {
    h: 0.57575756,
    s: 1.0,
    l: 0.47058824,
    a: 1.0,
};

impl Color {
    pub const BLACK: Color = Color {
        h: 0.,
        s: 0.,
        l: 0.,
        a: 1.,
    };
    pub const WHITE: Color = Color {
        h: 0.,
        s: 0.,
        l: 1.,
        a: 1.,
    };

    pub fn css(self) -> String {
        format!(
            "hsla({}, {}%, {}%, {})",
            (self.h * 360.).round(),
            (self.s * 100.).round(),
            (self.l * 100.).round(),
            self.a
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShadowLayer<C = Color> {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: C,
    pub inset: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tokens<C = Color> {
    pub background_modifier_active_hover: C,
    pub background_modifier_border: C,
    pub background_modifier_border_focus: C,
    pub background_modifier_border_hover: C,
    pub background_modifier_error: C,
    pub background_modifier_form_field: C,
    pub background_modifier_hover: C,
    pub background_primary: C,
    pub background_primary_alt: C,
    pub background_secondary: C,
    pub blockquote_border_color: C,
    pub canvas_background: C,
    pub canvas_card_label_color: C,
    pub canvas_color: C,
    pub canvas_color_1: C,
    pub canvas_color_2: C,
    pub canvas_color_3: C,
    pub canvas_color_4: C,
    pub canvas_color_5: C,
    pub canvas_color_6: C,
    pub canvas_dot_pattern: C,
    pub checkbox_border_color: C,
    pub checkbox_color: C,
    pub checkbox_marker_color: C,
    pub checklist_done_color: C,
    pub code_background: C,
    pub code_comment: C,
    pub code_function: C,
    pub code_important: C,
    pub code_keyword: C,
    pub code_normal: C,
    pub code_operator: C,
    pub code_property: C,
    pub code_punctuation: C,
    pub code_string: C,
    pub code_tag: C,
    pub code_value: C,
    pub collapse_icon_color: C,
    pub color_accent: C,
    pub color_blue: C,
    pub color_cyan: C,
    pub color_green: C,
    pub color_orange: C,
    pub color_pink: C,
    pub color_purple: C,
    pub color_red: C,
    pub color_yellow: C,
    pub divider_color: C,
    pub dropdown_background: C,
    pub graph_line: C,
    pub graph_node: C,
    pub graph_node_attachment: C,
    pub graph_node_focused: C,
    pub graph_node_tag: C,
    pub graph_node_unresolved: C,
    pub graph_text: C,
    pub highlight_background: C,
    pub hotkey_background: C,
    pub hr_color: C,
    pub icon_color: C,
    pub icon_color_active: C,
    pub icon_color_focused: C,
    pub icon_color_hover: C,
    pub icon_opacity: f32,
    pub icon_opacity_active: f32,
    pub icon_opacity_hover: f32,
    pub indentation_guide_color: C,
    pub indentation_guide_color_active: C,
    pub interactive_accent: C,
    pub interactive_accent_hover: C,
    pub interactive_hover: C,
    pub interactive_normal: C,
    pub link_color: C,
    pub list_marker_color: C,
    pub menu_background: C,
    pub menu_border_color: C,
    pub metadata_border_color: C,
    pub modal_background: C,
    pub modal_sidebar_background: C,
    pub nav_collapse_icon_color: C,
    pub nav_item_background_active: C,
    pub nav_item_background_hover: C,
    pub nav_item_color: C,
    pub nav_item_color_active: C,
    pub nav_item_color_hover: C,
    pub nav_tag_color: C,
    pub pdf_background: C,
    pub pdf_shadow: Vec<ShadowLayer<C>>,
    pub pdf_sidebar_background: C,
    pub pdf_thumbnail_shadow: Vec<ShadowLayer<C>>,
    pub pill_background: C,
    pub pill_border_color: C,
    pub pill_color: C,
    pub prompt_background: C,
    pub prompt_border_color: C,
    pub search_clear_button_color: C,
    pub search_icon_color: C,
    pub search_result_background: C,
    pub setting_item_name_color: C,
    pub shadow_s: Vec<ShadowLayer<C>>,
    pub sidebar_tab_container_background: C,
    pub slider_fill_background: C,
    pub slider_thumb_background: C,
    pub slider_track_background: C,
    pub suggestion_background: C,
    pub tab_background_active: C,
    pub tab_container_background: C,
    pub tab_divider_color: C,
    pub tab_inner_background_hover: C,
    pub tab_outline_color: C,
    pub tab_text_color_focused: C,
    pub tab_text_color_focused_active: C,
    pub tab_text_color_focused_active_current: C,
    pub table_border_color: C,
    pub table_header_background: C,
    pub table_header_border_color: C,
    pub table_header_color: C,
    pub tag_color: C,
    pub text_accent: C,
    pub text_accent_hover: C,
    pub text_error: C,
    pub text_faint: C,
    pub text_muted: C,
    pub text_normal: C,
    pub text_on_accent: C,
    pub text_selection: C,
    pub text_warning: C,
    pub toggle_thumb_color: C,
    pub tooltip_background: C,
    pub tooltip_color: C,
    pub vault_profile_color: C,
}

#[rustfmt::skip]
pub fn dark() -> Tokens<Color> {
    Tokens {
        background_modifier_active_hover: Hsla { h: 0.038889088, s: 0.62000054, l: 0.45999968, a: 0.1 },
        background_modifier_border: Hsla { h: 0.0, s: 0.0, l: 0.2, a: 1.0 },
        background_modifier_border_focus: Hsla { h: 0.0, s: 0.0, l: 0.33333334, a: 1.0 },
        background_modifier_border_hover: Hsla { h: 0.0, s: 0.0, l: 0.24705882, a: 1.0 },
        background_modifier_error: Hsla { h: 0.9944751, s: 0.95767194, l: 0.62941176, a: 1.0 },
        background_modifier_form_field: Hsla { h: 0.0, s: 0.0, l: 0.18039216, a: 1.0 },
        background_modifier_hover: Hsla { h: 0.0, s: 0.0, l: 0.99999994, a: 0.067 },
        background_primary: Hsla { h: 0.0, s: 0.0, l: 0.10980392, a: 1.0 },
        background_primary_alt: Hsla { h: 0.0, s: 0.0, l: 0.13725491, a: 1.0 },
        background_secondary: Hsla { h: 0.0, s: 0.0, l: 0.15686275, a: 1.0 },
        blockquote_border_color: PRIMARY_ACCENT,
        canvas_background: Hsla { h: 0.0, s: 0.0, l: 0.10980392, a: 1.0 },
        canvas_card_label_color: Hsla { h: 0.0, s: 0.0, l: 0.4, a: 1.0 },
        canvas_color: Hsla { h: 0.0, s: 0.0, l: 0.49411765, a: 1.0 },
        canvas_color_1: Hsla { h: 0.9944751, s: 0.95767194, l: 0.62941176, a: 1.0 },
        canvas_color_2: Hsla { h: 0.08627451, s: 0.7943926, l: 0.5803922, a: 1.0 },
        canvas_color_3: Hsla { h: 0.16366367, s: 0.6416185, l: 0.6607843, a: 1.0 },
        canvas_color_4: Hsla { h: 0.38369307, s: 0.5914894, l: 0.5392157, a: 1.0 },
        canvas_color_5: Hsla { h: 0.49761903, s: 0.6862745, l: 0.6, a: 1.0 },
        canvas_color_6: Hsla { h: 0.7173333, s: 1.0000001, l: 0.754902, a: 1.0 },
        canvas_dot_pattern: Hsla { h: 0.0, s: 0.0, l: 0.2, a: 1.0 },
        checkbox_border_color: Hsla { h: 0.0, s: 0.0, l: 0.4, a: 1.0 },
        checkbox_color: PRIMARY_ACCENT,
        checkbox_marker_color: Hsla { h: 0.0, s: 0.0, l: 0.10980392, a: 1.0 },
        checklist_done_color: Hsla { h: 0.0, s: 0.0, l: 0.7019608, a: 1.0 },
        code_background: Hsla { h: 0.0, s: 0.0, l: 0.13725491, a: 1.0 },
        code_comment: Hsla { h: 0.0, s: 0.0, l: 0.4, a: 1.0 },
        code_function: Hsla { h: 0.16366367, s: 0.6416185, l: 0.6607843, a: 1.0 },
        code_important: Hsla { h: 0.08627451, s: 0.7943926, l: 0.5803922, a: 1.0 },
        code_keyword: Hsla { h: 0.91065294, s: 0.90654194, l: 0.79019606, a: 1.0 },
        code_normal: Hsla { h: 0.0, s: 0.0, l: 0.85490197, a: 1.0 },
        code_operator: Hsla { h: 0.9944751, s: 0.95767194, l: 0.62941176, a: 1.0 },
        code_property: Hsla { h: 0.49761903, s: 0.6862745, l: 0.6, a: 1.0 },
        code_punctuation: Hsla { h: 0.0, s: 0.0, l: 0.7019608, a: 1.0 },
        code_string: Hsla { h: 0.38369307, s: 0.5914894, l: 0.5392157, a: 1.0 },
        code_tag: Hsla { h: 0.9944751, s: 0.95767194, l: 0.62941176, a: 1.0 },
        code_value: Hsla { h: 0.7173333, s: 1.0000001, l: 0.754902, a: 1.0 },
        collapse_icon_color: Hsla { h: 0.0, s: 0.0, l: 0.4, a: 1.0 },
        color_accent: PRIMARY_ACCENT,
        color_blue: Hsla { h: 0.5876153, s: 1.0, l: 0.50392157, a: 1.0 },
        color_cyan: Hsla { h: 0.49761903, s: 0.6862745, l: 0.6, a: 1.0 },
        color_green: Hsla { h: 0.38369307, s: 0.5914894, l: 0.5392157, a: 1.0 },
        color_orange: Hsla { h: 0.08627451, s: 0.7943926, l: 0.5803922, a: 1.0 },
        color_pink: Hsla { h: 0.91065294, s: 0.90654194, l: 0.79019606, a: 1.0 },
        color_purple: Hsla { h: 0.7173333, s: 1.0000001, l: 0.754902, a: 1.0 },
        color_red: Hsla { h: 0.9944751, s: 0.95767194, l: 0.62941176, a: 1.0 },
        color_yellow: Hsla { h: 0.16366367, s: 0.6416185, l: 0.6607843, a: 1.0 },
        divider_color: Hsla { h: 0.0, s: 0.0, l: 0.2, a: 1.0 },
        dropdown_background: Hsla { h: 0.0, s: 0.0, l: 0.2, a: 1.0 },
        graph_line: Hsla { h: 0.0, s: 0.0, l: 0.24705882, a: 1.0 },
        graph_node: Hsla { h: 0.0, s: 0.0, l: 0.7019608, a: 1.0 },
        graph_node_attachment: Hsla { h: 0.16366367, s: 0.6416185, l: 0.6607843, a: 1.0 },
        graph_node_focused: PRIMARY_ACCENT,
        graph_node_tag: Hsla { h: 0.38369307, s: 0.5914894, l: 0.5392157, a: 1.0 },
        graph_node_unresolved: Hsla { h: 0.0, s: 0.0, l: 0.4, a: 1.0 },
        graph_text: Hsla { h: 0.0, s: 0.0, l: 0.85490197, a: 1.0 },
        highlight_background: Hsla { h: 0.16366369, s: 0.64161855, l: 0.6607841, a: 0.3 },
        hotkey_background: Hsla { h: 0.0, s: 0.0, l: 0.99999994, a: 0.067 },
        hr_color: Hsla { h: 0.0, s: 0.0, l: 0.2, a: 1.0 },
        icon_color: Hsla { h: 0.0, s: 0.0, l: 0.7019608, a: 1.0 },
        icon_color_active: PRIMARY_ACCENT,
        icon_color_focused: Hsla { h: 0.0, s: 0.0, l: 0.85490197, a: 1.0 },
        icon_color_hover: Hsla { h: 0.0, s: 0.0, l: 0.7019608, a: 1.0 },
        icon_opacity: 0.85,
        icon_opacity_active: 1.0,
        icon_opacity_hover: 1.0,
        indentation_guide_color: Hsla { h: 0.0, s: 0.0, l: 0.99999994, a: 0.12 },
        indentation_guide_color_active: Hsla { h: 0.0, s: 0.0, l: 0.99999994, a: 0.3 },
        interactive_accent: PRIMARY_ACCENT,
        interactive_accent_hover: PRIMARY_ACCENT,
        interactive_hover: Hsla { h: 0.0, s: 0.0, l: 0.24705882, a: 1.0 },
        interactive_normal: Hsla { h: 0.0, s: 0.0, l: 0.2, a: 1.0 },
        link_color: PRIMARY_ACCENT,
        list_marker_color: Hsla { h: 0.0, s: 0.0, l: 0.4, a: 1.0 },
        menu_background: Hsla { h: 0.0, s: 0.0, l: 0.15686275, a: 1.0 },
        menu_border_color: Hsla { h: 0.0, s: 0.0, l: 0.24705882, a: 1.0 },
        metadata_border_color: Hsla { h: 0.0, s: 0.0, l: 0.2, a: 1.0 },
        modal_background: Hsla { h: 0.0, s: 0.0, l: 0.10980392, a: 1.0 },
        modal_sidebar_background: Hsla { h: 0.0, s: 0.0, l: 0.10980392, a: 1.0 },
        nav_collapse_icon_color: Hsla { h: 0.0, s: 0.0, l: 0.4, a: 1.0 },
        nav_item_background_active: Hsla { h: 0.0, s: 0.0, l: 0.99999994, a: 0.067 },
        nav_item_background_hover: Hsla { h: 0.0, s: 0.0, l: 0.99999994, a: 0.067 },
        nav_item_color: Hsla { h: 0.0, s: 0.0, l: 0.7019608, a: 1.0 },
        nav_item_color_active: Hsla { h: 0.0, s: 0.0, l: 0.85490197, a: 1.0 },
        nav_item_color_hover: Hsla { h: 0.0, s: 0.0, l: 0.85490197, a: 1.0 },
        nav_tag_color: Hsla { h: 0.0, s: 0.0, l: 0.4, a: 1.0 },
        pdf_background: Hsla { h: 0.0, s: 0.0, l: 0.10980392, a: 1.0 },
        pdf_shadow: vec![ShadowLayer { x: 0.0, y: 0.0, blur: 0.0, spread: 1.0, color: Hsla { h: 0.0, s: 0.0, l: 0.2, a: 1.0 }, inset: false }, ],
        pdf_sidebar_background: Hsla { h: 0.0, s: 0.0, l: 0.10980392, a: 1.0 },
        pdf_thumbnail_shadow: vec![ShadowLayer { x: 0.0, y: 0.0, blur: 0.0, spread: 1.0, color: Hsla { h: 0.0, s: 0.0, l: 0.2, a: 1.0 }, inset: false }, ],
        pill_background: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.0 },
        pill_border_color: Hsla { h: 0.0, s: 0.0, l: 0.2, a: 1.0 },
        pill_color: Hsla { h: 0.0, s: 0.0, l: 0.7019608, a: 1.0 },
        prompt_background: Hsla { h: 0.0, s: 0.0, l: 0.10980392, a: 1.0 },
        prompt_border_color: Hsla { h: 0.0, s: 0.0, l: 0.24705882, a: 1.0 },
        search_clear_button_color: Hsla { h: 0.0, s: 0.0, l: 0.7019608, a: 1.0 },
        search_icon_color: Hsla { h: 0.0, s: 0.0, l: 0.7019608, a: 1.0 },
        search_result_background: Hsla { h: 0.0, s: 0.0, l: 0.10980392, a: 1.0 },
        setting_item_name_color: Hsla { h: 0.0, s: 0.0, l: 0.85490197, a: 1.0 },
        shadow_s: vec![ShadowLayer { x: 0.0, y: 1.0, blur: 2.0, spread: 0.0, color: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.121 }, inset: false }, ShadowLayer { x: 0.0, y: 3.4, blur: 6.7, spread: 0.0, color: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.179 }, inset: false }, ShadowLayer { x: 0.0, y: 15.0, blur: 30.0, spread: 0.0, color: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.3 }, inset: false }, ],
        sidebar_tab_container_background: Hsla { h: 0.0, s: 0.0, l: 0.15686275, a: 1.0 },
        slider_fill_background: PRIMARY_ACCENT,
        slider_thumb_background: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        slider_track_background: Hsla { h: 0.0, s: 0.0, l: 0.24705882, a: 1.0 },
        suggestion_background: Hsla { h: 0.0, s: 0.0, l: 0.10980392, a: 1.0 },
        tab_background_active: Hsla { h: 0.0, s: 0.0, l: 0.10980392, a: 1.0 },
        tab_container_background: Hsla { h: 0.0, s: 0.0, l: 0.15686275, a: 1.0 },
        tab_divider_color: Hsla { h: 0.0, s: 0.0, l: 0.24705882, a: 1.0 },
        tab_inner_background_hover: Hsla { h: 0.0, s: 0.0, l: 0.99999994, a: 0.067 },
        tab_outline_color: Hsla { h: 0.0, s: 0.0, l: 0.2, a: 1.0 },
        tab_text_color_focused: Hsla { h: 0.0, s: 0.0, l: 0.7019608, a: 1.0 },
        tab_text_color_focused_active: Hsla { h: 0.0, s: 0.0, l: 0.7019608, a: 1.0 },
        tab_text_color_focused_active_current: Hsla { h: 0.0, s: 0.0, l: 0.85490197, a: 1.0 },
        table_border_color: Hsla { h: 0.0, s: 0.0, l: 0.2, a: 1.0 },
        table_header_background: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.0 },
        table_header_border_color: Hsla { h: 0.0, s: 0.0, l: 0.2, a: 1.0 },
        table_header_color: Hsla { h: 0.0, s: 0.0, l: 0.85490197, a: 1.0 },
        tag_color: PRIMARY_ACCENT,
        text_accent: PRIMARY_ACCENT,
        text_accent_hover: PRIMARY_ACCENT,
        text_error: Hsla { h: 0.9944751, s: 0.95767194, l: 0.62941176, a: 1.0 },
        text_faint: Hsla { h: 0.0, s: 0.0, l: 0.4, a: 1.0 },
        text_muted: Hsla { h: 0.0, s: 0.0, l: 0.7019608, a: 1.0 },
        text_normal: Hsla { h: 0.0, s: 0.0, l: 0.85490197, a: 1.0 },
        text_on_accent: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        text_selection: Hsla { a: 0.33, ..PRIMARY_ACCENT },
        text_warning: Hsla { h: 0.08627451, s: 0.7943926, l: 0.5803922, a: 1.0 },
        toggle_thumb_color: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        tooltip_background: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.9 },
        tooltip_color: Hsla { h: 0.0, s: 0.0, l: 0.98039216, a: 1.0 },
        vault_profile_color: Hsla { h: 0.0, s: 0.0, l: 0.85490197, a: 1.0 },
    }
}
#[rustfmt::skip]
pub fn light() -> Tokens<Color> {
    Tokens {
        background_modifier_active_hover: Hsla { h: 0.036111046, s: 0.62619954, l: 0.4945001, a: 0.1 },
        background_modifier_border: Hsla { h: 0.0, s: 0.0, l: 0.89411765, a: 1.0 },
        background_modifier_border_focus: Hsla { h: 0.0, s: 0.0, l: 0.7411765, a: 1.0 },
        background_modifier_border_hover: Hsla { h: 0.0, s: 0.0, l: 0.85490197, a: 1.0 },
        background_modifier_error: Hsla { h: 0.9800725, s: 0.8070176, l: 0.5529412, a: 1.0 },
        background_modifier_form_field: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        background_modifier_hover: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.067 },
        background_primary: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        background_primary_alt: Hsla { h: 0.0, s: 0.0, l: 0.98039216, a: 1.0 },
        background_secondary: Hsla { h: 0.0, s: 0.0, l: 0.9647059, a: 1.0 },
        blockquote_border_color: PRIMARY_ACCENT,
        canvas_background: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        canvas_card_label_color: Hsla { h: 0.0, s: 0.0, l: 0.67058825, a: 1.0 },
        canvas_color: Hsla { h: 0.0, s: 0.0, l: 0.7529412, a: 1.0 },
        canvas_color_1: Hsla { h: 0.9800725, s: 0.8070176, l: 0.5529412, a: 1.0 },
        canvas_color_2: Hsla { h: 0.08262712, s: 1.0, l: 0.4627451, a: 1.0 },
        canvas_color_3: Hsla { h: 0.1279762, s: 1.0, l: 0.4392157, a: 1.0 },
        canvas_color_4: Hsla { h: 0.3992467, s: 0.91709846, l: 0.37843138, a: 1.0 },
        canvas_color_5: Hsla { h: 0.4973822, s: 1.0, l: 0.3745098, a: 1.0 },
        canvas_color_6: Hsla { h: 0.70726496, s: 0.8210526, l: 0.627451, a: 1.0 },
        canvas_dot_pattern: Hsla { h: 0.0, s: 0.0, l: 0.89411765, a: 1.0 },
        checkbox_border_color: Hsla { h: 0.0, s: 0.0, l: 0.67058825, a: 1.0 },
        checkbox_color: PRIMARY_ACCENT,
        checkbox_marker_color: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        checklist_done_color: Hsla { h: 0.0, s: 0.0, l: 0.36078432, a: 1.0 },
        code_background: Hsla { h: 0.0, s: 0.0, l: 0.98039216, a: 1.0 },
        code_comment: Hsla { h: 0.0, s: 0.0, l: 0.67058825, a: 1.0 },
        code_function: Hsla { h: 0.1279762, s: 1.0, l: 0.4392157, a: 1.0 },
        code_important: Hsla { h: 0.08262712, s: 1.0, l: 0.4627451, a: 1.0 },
        code_keyword: Hsla { h: 0.9198718, s: 0.65000004, l: 0.5294118, a: 1.0 },
        code_normal: Hsla { h: 0.0, s: 0.0, l: 0.13333334, a: 1.0 },
        code_operator: Hsla { h: 0.9800725, s: 0.8070176, l: 0.5529412, a: 1.0 },
        code_property: Hsla { h: 0.4973822, s: 1.0, l: 0.3745098, a: 1.0 },
        code_punctuation: Hsla { h: 0.0, s: 0.0, l: 0.36078432, a: 1.0 },
        code_string: Hsla { h: 0.3992467, s: 0.91709846, l: 0.37843138, a: 1.0 },
        code_tag: Hsla { h: 0.9800725, s: 0.8070176, l: 0.5529412, a: 1.0 },
        code_value: Hsla { h: 0.70726496, s: 0.8210526, l: 0.627451, a: 1.0 },
        collapse_icon_color: Hsla { h: 0.0, s: 0.0, l: 0.67058825, a: 1.0 },
        color_accent: PRIMARY_ACCENT,
        color_blue: Hsla { h: 0.58763695, s: 0.930131, l: 0.4490196, a: 1.0 },
        color_cyan: Hsla { h: 0.4973822, s: 1.0, l: 0.3745098, a: 1.0 },
        color_green: Hsla { h: 0.3992467, s: 0.91709846, l: 0.37843138, a: 1.0 },
        color_orange: Hsla { h: 0.08262712, s: 1.0, l: 0.4627451, a: 1.0 },
        color_pink: Hsla { h: 0.9198718, s: 0.65000004, l: 0.5294118, a: 1.0 },
        color_purple: Hsla { h: 0.70726496, s: 0.8210526, l: 0.627451, a: 1.0 },
        color_red: Hsla { h: 0.9800725, s: 0.8070176, l: 0.5529412, a: 1.0 },
        color_yellow: Hsla { h: 0.1279762, s: 1.0, l: 0.4392157, a: 1.0 },
        divider_color: Hsla { h: 0.0, s: 0.0, l: 0.89411765, a: 1.0 },
        dropdown_background: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        graph_line: Hsla { h: 0.0, s: 0.0, l: 0.85490197, a: 1.0 },
        graph_node: Hsla { h: 0.0, s: 0.0, l: 0.36078432, a: 1.0 },
        graph_node_attachment: Hsla { h: 0.1279762, s: 1.0, l: 0.4392157, a: 1.0 },
        graph_node_focused: PRIMARY_ACCENT,
        graph_node_tag: Hsla { h: 0.3992467, s: 0.91709846, l: 0.37843138, a: 1.0 },
        graph_node_unresolved: Hsla { h: 0.0, s: 0.0, l: 0.67058825, a: 1.0 },
        graph_text: Hsla { h: 0.0, s: 0.0, l: 0.13333334, a: 1.0 },
        highlight_background: Hsla { h: 0.1279762, s: 1.0, l: 0.43921575, a: 0.3 },
        hotkey_background: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.067 },
        hr_color: Hsla { h: 0.0, s: 0.0, l: 0.89411765, a: 1.0 },
        icon_color: Hsla { h: 0.0, s: 0.0, l: 0.36078432, a: 1.0 },
        icon_color_active: PRIMARY_ACCENT,
        icon_color_focused: Hsla { h: 0.0, s: 0.0, l: 0.13333334, a: 1.0 },
        icon_color_hover: Hsla { h: 0.0, s: 0.0, l: 0.36078432, a: 1.0 },
        icon_opacity: 0.85,
        icon_opacity_active: 1.0,
        icon_opacity_hover: 1.0,
        indentation_guide_color: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.12 },
        indentation_guide_color_active: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.3 },
        interactive_accent: PRIMARY_ACCENT,
        interactive_accent_hover: PRIMARY_ACCENT,
        interactive_hover: Hsla { h: 0.0, s: 0.0, l: 0.98039216, a: 1.0 },
        interactive_normal: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        link_color: PRIMARY_ACCENT,
        list_marker_color: Hsla { h: 0.0, s: 0.0, l: 0.67058825, a: 1.0 },
        menu_background: Hsla { h: 0.0, s: 0.0, l: 0.9647059, a: 1.0 },
        menu_border_color: Hsla { h: 0.0, s: 0.0, l: 0.85490197, a: 1.0 },
        metadata_border_color: Hsla { h: 0.0, s: 0.0, l: 0.89411765, a: 1.0 },
        modal_background: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        modal_sidebar_background: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        nav_collapse_icon_color: Hsla { h: 0.0, s: 0.0, l: 0.67058825, a: 1.0 },
        nav_item_background_active: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.067 },
        nav_item_background_hover: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.067 },
        nav_item_color: Hsla { h: 0.0, s: 0.0, l: 0.36078432, a: 1.0 },
        nav_item_color_active: Hsla { h: 0.0, s: 0.0, l: 0.13333334, a: 1.0 },
        nav_item_color_hover: Hsla { h: 0.0, s: 0.0, l: 0.13333334, a: 1.0 },
        nav_tag_color: Hsla { h: 0.0, s: 0.0, l: 0.67058825, a: 1.0 },
        pdf_background: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        pdf_shadow: vec![ShadowLayer { x: 0.0, y: 0.0, blur: 0.0, spread: 1.0, color: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.05 }, inset: false }, ShadowLayer { x: 0.0, y: 2.0, blur: 8.0, spread: 0.0, color: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.1 }, inset: false }, ],
        pdf_sidebar_background: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        pdf_thumbnail_shadow: vec![ShadowLayer { x: 0.0, y: 0.0, blur: 0.0, spread: 1.0, color: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.15 }, inset: false }, ShadowLayer { x: 0.0, y: 2.0, blur: 8.0, spread: 0.0, color: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.2 }, inset: false }, ],
        pill_background: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.0 },
        pill_border_color: Hsla { h: 0.0, s: 0.0, l: 0.89411765, a: 1.0 },
        pill_color: Hsla { h: 0.0, s: 0.0, l: 0.36078432, a: 1.0 },
        prompt_background: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        prompt_border_color: Hsla { h: 0.0, s: 0.0, l: 0.85490197, a: 1.0 },
        search_clear_button_color: Hsla { h: 0.0, s: 0.0, l: 0.36078432, a: 1.0 },
        search_icon_color: Hsla { h: 0.0, s: 0.0, l: 0.36078432, a: 1.0 },
        search_result_background: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        setting_item_name_color: Hsla { h: 0.0, s: 0.0, l: 0.13333334, a: 1.0 },
        shadow_s: vec![ShadowLayer { x: 0.0, y: 1.0, blur: 2.0, spread: 0.0, color: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.028 }, inset: false }, ShadowLayer { x: 0.0, y: 3.4, blur: 6.7, spread: 0.0, color: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.042 }, inset: false }, ShadowLayer { x: 0.0, y: 15.0, blur: 30.0, spread: 0.0, color: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.07 }, inset: false }, ],
        sidebar_tab_container_background: Hsla { h: 0.0, s: 0.0, l: 0.9647059, a: 1.0 },
        slider_fill_background: PRIMARY_ACCENT,
        slider_thumb_background: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        slider_track_background: Hsla { h: 0.0, s: 0.0, l: 0.85490197, a: 1.0 },
        suggestion_background: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        tab_background_active: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        tab_container_background: Hsla { h: 0.0, s: 0.0, l: 0.9647059, a: 1.0 },
        tab_divider_color: Hsla { h: 0.0, s: 0.0, l: 0.85490197, a: 1.0 },
        tab_inner_background_hover: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.067 },
        tab_outline_color: Hsla { h: 0.0, s: 0.0, l: 0.89411765, a: 1.0 },
        tab_text_color_focused: Hsla { h: 0.0, s: 0.0, l: 0.36078432, a: 1.0 },
        tab_text_color_focused_active: Hsla { h: 0.0, s: 0.0, l: 0.36078432, a: 1.0 },
        tab_text_color_focused_active_current: Hsla { h: 0.0, s: 0.0, l: 0.13333334, a: 1.0 },
        table_border_color: Hsla { h: 0.0, s: 0.0, l: 0.89411765, a: 1.0 },
        table_header_background: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.0 },
        table_header_border_color: Hsla { h: 0.0, s: 0.0, l: 0.89411765, a: 1.0 },
        table_header_color: Hsla { h: 0.0, s: 0.0, l: 0.13333334, a: 1.0 },
        tag_color: PRIMARY_ACCENT,
        text_accent: PRIMARY_ACCENT,
        text_accent_hover: PRIMARY_ACCENT,
        text_error: Hsla { h: 0.9800725, s: 0.8070176, l: 0.5529412, a: 1.0 },
        text_faint: Hsla { h: 0.0, s: 0.0, l: 0.67058825, a: 1.0 },
        text_muted: Hsla { h: 0.0, s: 0.0, l: 0.36078432, a: 1.0 },
        text_normal: Hsla { h: 0.0, s: 0.0, l: 0.13333334, a: 1.0 },
        text_on_accent: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        text_selection: Hsla { a: 0.2, ..PRIMARY_ACCENT },
        text_warning: Hsla { h: 0.08262712, s: 1.0, l: 0.4627451, a: 1.0 },
        toggle_thumb_color: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 },
        tooltip_background: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.9 },
        tooltip_color: Hsla { h: 0.0, s: 0.0, l: 0.98039216, a: 1.0 },
        vault_profile_color: Hsla { h: 0.0, s: 0.0, l: 0.13333334, a: 1.0 },
    }
}

impl<C> Tokens<C> {
    pub fn map<D>(self, f: impl Fn(C) -> D + Copy) -> Tokens<D> {
        Tokens {
            background_modifier_active_hover: f(self.background_modifier_active_hover),
            background_modifier_border: f(self.background_modifier_border),
            background_modifier_border_focus: f(self.background_modifier_border_focus),
            background_modifier_border_hover: f(self.background_modifier_border_hover),
            background_modifier_error: f(self.background_modifier_error),
            background_modifier_form_field: f(self.background_modifier_form_field),
            background_modifier_hover: f(self.background_modifier_hover),
            background_primary: f(self.background_primary),
            background_primary_alt: f(self.background_primary_alt),
            background_secondary: f(self.background_secondary),
            blockquote_border_color: f(self.blockquote_border_color),
            canvas_background: f(self.canvas_background),
            canvas_card_label_color: f(self.canvas_card_label_color),
            canvas_color: f(self.canvas_color),
            canvas_color_1: f(self.canvas_color_1),
            canvas_color_2: f(self.canvas_color_2),
            canvas_color_3: f(self.canvas_color_3),
            canvas_color_4: f(self.canvas_color_4),
            canvas_color_5: f(self.canvas_color_5),
            canvas_color_6: f(self.canvas_color_6),
            canvas_dot_pattern: f(self.canvas_dot_pattern),
            checkbox_border_color: f(self.checkbox_border_color),
            checkbox_color: f(self.checkbox_color),
            checkbox_marker_color: f(self.checkbox_marker_color),
            checklist_done_color: f(self.checklist_done_color),
            code_background: f(self.code_background),
            code_comment: f(self.code_comment),
            code_function: f(self.code_function),
            code_important: f(self.code_important),
            code_keyword: f(self.code_keyword),
            code_normal: f(self.code_normal),
            code_operator: f(self.code_operator),
            code_property: f(self.code_property),
            code_punctuation: f(self.code_punctuation),
            code_string: f(self.code_string),
            code_tag: f(self.code_tag),
            code_value: f(self.code_value),
            collapse_icon_color: f(self.collapse_icon_color),
            color_accent: f(self.color_accent),
            color_blue: f(self.color_blue),
            color_cyan: f(self.color_cyan),
            color_green: f(self.color_green),
            color_orange: f(self.color_orange),
            color_pink: f(self.color_pink),
            color_purple: f(self.color_purple),
            color_red: f(self.color_red),
            color_yellow: f(self.color_yellow),
            divider_color: f(self.divider_color),
            dropdown_background: f(self.dropdown_background),
            graph_line: f(self.graph_line),
            graph_node: f(self.graph_node),
            graph_node_attachment: f(self.graph_node_attachment),
            graph_node_focused: f(self.graph_node_focused),
            graph_node_tag: f(self.graph_node_tag),
            graph_node_unresolved: f(self.graph_node_unresolved),
            graph_text: f(self.graph_text),
            highlight_background: f(self.highlight_background),
            hotkey_background: f(self.hotkey_background),
            hr_color: f(self.hr_color),
            icon_color: f(self.icon_color),
            icon_color_active: f(self.icon_color_active),
            icon_color_focused: f(self.icon_color_focused),
            icon_color_hover: f(self.icon_color_hover),
            indentation_guide_color: f(self.indentation_guide_color),
            indentation_guide_color_active: f(self.indentation_guide_color_active),
            interactive_accent: f(self.interactive_accent),
            interactive_accent_hover: f(self.interactive_accent_hover),
            interactive_hover: f(self.interactive_hover),
            interactive_normal: f(self.interactive_normal),
            link_color: f(self.link_color),
            list_marker_color: f(self.list_marker_color),
            menu_background: f(self.menu_background),
            menu_border_color: f(self.menu_border_color),
            metadata_border_color: f(self.metadata_border_color),
            modal_background: f(self.modal_background),
            modal_sidebar_background: f(self.modal_sidebar_background),
            nav_collapse_icon_color: f(self.nav_collapse_icon_color),
            nav_item_background_active: f(self.nav_item_background_active),
            nav_item_background_hover: f(self.nav_item_background_hover),
            nav_item_color: f(self.nav_item_color),
            nav_item_color_active: f(self.nav_item_color_active),
            nav_item_color_hover: f(self.nav_item_color_hover),
            nav_tag_color: f(self.nav_tag_color),
            pdf_background: f(self.pdf_background),
            pdf_sidebar_background: f(self.pdf_sidebar_background),
            pill_background: f(self.pill_background),
            pill_border_color: f(self.pill_border_color),
            pill_color: f(self.pill_color),
            prompt_background: f(self.prompt_background),
            prompt_border_color: f(self.prompt_border_color),
            search_clear_button_color: f(self.search_clear_button_color),
            search_icon_color: f(self.search_icon_color),
            search_result_background: f(self.search_result_background),
            setting_item_name_color: f(self.setting_item_name_color),
            sidebar_tab_container_background: f(self.sidebar_tab_container_background),
            slider_fill_background: f(self.slider_fill_background),
            slider_thumb_background: f(self.slider_thumb_background),
            slider_track_background: f(self.slider_track_background),
            suggestion_background: f(self.suggestion_background),
            tab_background_active: f(self.tab_background_active),
            tab_container_background: f(self.tab_container_background),
            tab_divider_color: f(self.tab_divider_color),
            tab_inner_background_hover: f(self.tab_inner_background_hover),
            tab_outline_color: f(self.tab_outline_color),
            tab_text_color_focused: f(self.tab_text_color_focused),
            tab_text_color_focused_active: f(self.tab_text_color_focused_active),
            tab_text_color_focused_active_current: f(self.tab_text_color_focused_active_current),
            table_border_color: f(self.table_border_color),
            table_header_background: f(self.table_header_background),
            table_header_border_color: f(self.table_header_border_color),
            table_header_color: f(self.table_header_color),
            tag_color: f(self.tag_color),
            text_accent: f(self.text_accent),
            text_accent_hover: f(self.text_accent_hover),
            text_error: f(self.text_error),
            text_faint: f(self.text_faint),
            text_muted: f(self.text_muted),
            text_normal: f(self.text_normal),
            text_on_accent: f(self.text_on_accent),
            text_selection: f(self.text_selection),
            text_warning: f(self.text_warning),
            toggle_thumb_color: f(self.toggle_thumb_color),
            tooltip_background: f(self.tooltip_background),
            tooltip_color: f(self.tooltip_color),
            vault_profile_color: f(self.vault_profile_color),
            icon_opacity: self.icon_opacity,
            icon_opacity_active: self.icon_opacity_active,
            icon_opacity_hover: self.icon_opacity_hover,
            pdf_shadow: self.pdf_shadow.into_iter().map(|s| s.map(f)).collect(),
            pdf_thumbnail_shadow: self
                .pdf_thumbnail_shadow
                .into_iter()
                .map(|s| s.map(f))
                .collect(),
            shadow_s: self.shadow_s.into_iter().map(|s| s.map(f)).collect(),
        }
    }
}

impl<C> ShadowLayer<C> {
    pub fn map<D>(self, f: impl Fn(C) -> D + Copy) -> ShadowLayer<D> {
        ShadowLayer {
            x: self.x,
            y: self.y,
            blur: self.blur,
            spread: self.spread,
            color: f(self.color),
            inset: self.inset,
        }
    }
}

impl Tokens<Color> {
    pub fn css_vars(&self) -> String {
        let mut css = String::from(":root{");
        css.push_str(&format!(
            "--background-modifier-active-hover:{};",
            self.background_modifier_active_hover.css()
        ));
        css.push_str(&format!(
            "--background-modifier-border:{};",
            self.background_modifier_border.css()
        ));
        css.push_str(&format!(
            "--background-modifier-border-focus:{};",
            self.background_modifier_border_focus.css()
        ));
        css.push_str(&format!(
            "--background-modifier-border-hover:{};",
            self.background_modifier_border_hover.css()
        ));
        css.push_str(&format!(
            "--background-modifier-error:{};",
            self.background_modifier_error.css()
        ));
        css.push_str(&format!(
            "--background-modifier-form-field:{};",
            self.background_modifier_form_field.css()
        ));
        css.push_str(&format!(
            "--background-modifier-hover:{};",
            self.background_modifier_hover.css()
        ));
        css.push_str(&format!(
            "--background-primary:{};",
            self.background_primary.css()
        ));
        css.push_str(&format!(
            "--background-primary-alt:{};",
            self.background_primary_alt.css()
        ));
        css.push_str(&format!(
            "--background-secondary:{};",
            self.background_secondary.css()
        ));
        css.push_str(&format!(
            "--blockquote-border-color:{};",
            self.blockquote_border_color.css()
        ));
        css.push_str(&format!(
            "--canvas-background:{};",
            self.canvas_background.css()
        ));
        css.push_str(&format!(
            "--canvas-card-label-color:{};",
            self.canvas_card_label_color.css()
        ));
        css.push_str(&format!("--canvas-color:{};", self.canvas_color.css()));
        css.push_str(&format!("--canvas-color-1:{};", self.canvas_color_1.css()));
        css.push_str(&format!("--canvas-color-2:{};", self.canvas_color_2.css()));
        css.push_str(&format!("--canvas-color-3:{};", self.canvas_color_3.css()));
        css.push_str(&format!("--canvas-color-4:{};", self.canvas_color_4.css()));
        css.push_str(&format!("--canvas-color-5:{};", self.canvas_color_5.css()));
        css.push_str(&format!("--canvas-color-6:{};", self.canvas_color_6.css()));
        css.push_str(&format!(
            "--canvas-dot-pattern:{};",
            self.canvas_dot_pattern.css()
        ));
        css.push_str(&format!(
            "--checkbox-border-color:{};",
            self.checkbox_border_color.css()
        ));
        css.push_str(&format!("--checkbox-color:{};", self.checkbox_color.css()));
        css.push_str(&format!(
            "--checkbox-marker-color:{};",
            self.checkbox_marker_color.css()
        ));
        css.push_str(&format!(
            "--checklist-done-color:{};",
            self.checklist_done_color.css()
        ));
        css.push_str(&format!(
            "--code-background:{};",
            self.code_background.css()
        ));
        css.push_str(&format!("--code-comment:{};", self.code_comment.css()));
        css.push_str(&format!("--code-function:{};", self.code_function.css()));
        css.push_str(&format!("--code-important:{};", self.code_important.css()));
        css.push_str(&format!("--code-keyword:{};", self.code_keyword.css()));
        css.push_str(&format!("--code-normal:{};", self.code_normal.css()));
        css.push_str(&format!("--code-operator:{};", self.code_operator.css()));
        css.push_str(&format!("--code-property:{};", self.code_property.css()));
        css.push_str(&format!(
            "--code-punctuation:{};",
            self.code_punctuation.css()
        ));
        css.push_str(&format!("--code-string:{};", self.code_string.css()));
        css.push_str(&format!("--code-tag:{};", self.code_tag.css()));
        css.push_str(&format!("--code-value:{};", self.code_value.css()));
        css.push_str(&format!(
            "--collapse-icon-color:{};",
            self.collapse_icon_color.css()
        ));
        css.push_str(&format!("--color-accent:{};", self.color_accent.css()));
        css.push_str(&format!("--color-blue:{};", self.color_blue.css()));
        css.push_str(&format!("--color-cyan:{};", self.color_cyan.css()));
        css.push_str(&format!("--color-green:{};", self.color_green.css()));
        css.push_str(&format!("--color-orange:{};", self.color_orange.css()));
        css.push_str(&format!("--color-pink:{};", self.color_pink.css()));
        css.push_str(&format!("--color-purple:{};", self.color_purple.css()));
        css.push_str(&format!("--color-red:{};", self.color_red.css()));
        css.push_str(&format!("--color-yellow:{};", self.color_yellow.css()));
        css.push_str(&format!("--divider-color:{};", self.divider_color.css()));
        css.push_str(&format!(
            "--dropdown-background:{};",
            self.dropdown_background.css()
        ));
        css.push_str(&format!("--graph-line:{};", self.graph_line.css()));
        css.push_str(&format!("--graph-node:{};", self.graph_node.css()));
        css.push_str(&format!(
            "--graph-node-attachment:{};",
            self.graph_node_attachment.css()
        ));
        css.push_str(&format!(
            "--graph-node-focused:{};",
            self.graph_node_focused.css()
        ));
        css.push_str(&format!("--graph-node-tag:{};", self.graph_node_tag.css()));
        css.push_str(&format!(
            "--graph-node-unresolved:{};",
            self.graph_node_unresolved.css()
        ));
        css.push_str(&format!("--graph-text:{};", self.graph_text.css()));
        css.push_str(&format!(
            "--highlight-background:{};",
            self.highlight_background.css()
        ));
        css.push_str(&format!(
            "--hotkey-background:{};",
            self.hotkey_background.css()
        ));
        css.push_str(&format!("--hr-color:{};", self.hr_color.css()));
        css.push_str(&format!("--icon-color:{};", self.icon_color.css()));
        css.push_str(&format!(
            "--icon-color-active:{};",
            self.icon_color_active.css()
        ));
        css.push_str(&format!(
            "--icon-color-focused:{};",
            self.icon_color_focused.css()
        ));
        css.push_str(&format!(
            "--icon-color-hover:{};",
            self.icon_color_hover.css()
        ));
        css.push_str(&format!("--icon-opacity:{};", self.icon_opacity));
        css.push_str(&format!(
            "--icon-opacity-active:{};",
            self.icon_opacity_active
        ));
        css.push_str(&format!(
            "--icon-opacity-hover:{};",
            self.icon_opacity_hover
        ));
        css.push_str(&format!(
            "--indentation-guide-color:{};",
            self.indentation_guide_color.css()
        ));
        css.push_str(&format!(
            "--indentation-guide-color-active:{};",
            self.indentation_guide_color_active.css()
        ));
        css.push_str(&format!(
            "--interactive-accent:{};",
            self.interactive_accent.css()
        ));
        css.push_str(&format!(
            "--interactive-accent-hover:{};",
            self.interactive_accent_hover.css()
        ));
        css.push_str(&format!(
            "--interactive-hover:{};",
            self.interactive_hover.css()
        ));
        css.push_str(&format!(
            "--interactive-normal:{};",
            self.interactive_normal.css()
        ));
        css.push_str(&format!("--link-color:{};", self.link_color.css()));
        css.push_str(&format!(
            "--list-marker-color:{};",
            self.list_marker_color.css()
        ));
        css.push_str(&format!(
            "--menu-background:{};",
            self.menu_background.css()
        ));
        css.push_str(&format!(
            "--menu-border-color:{};",
            self.menu_border_color.css()
        ));
        css.push_str(&format!(
            "--metadata-border-color:{};",
            self.metadata_border_color.css()
        ));
        css.push_str(&format!(
            "--modal-background:{};",
            self.modal_background.css()
        ));
        css.push_str(&format!(
            "--modal-sidebar-background:{};",
            self.modal_sidebar_background.css()
        ));
        css.push_str(&format!(
            "--nav-collapse-icon-color:{};",
            self.nav_collapse_icon_color.css()
        ));
        css.push_str(&format!(
            "--nav-item-background-active:{};",
            self.nav_item_background_active.css()
        ));
        css.push_str(&format!(
            "--nav-item-background-hover:{};",
            self.nav_item_background_hover.css()
        ));
        css.push_str(&format!("--nav-item-color:{};", self.nav_item_color.css()));
        css.push_str(&format!(
            "--nav-item-color-active:{};",
            self.nav_item_color_active.css()
        ));
        css.push_str(&format!(
            "--nav-item-color-hover:{};",
            self.nav_item_color_hover.css()
        ));
        css.push_str(&format!("--nav-tag-color:{};", self.nav_tag_color.css()));
        css.push_str(&format!("--pdf-background:{};", self.pdf_background.css()));
        css.push_str(&format!(
            "--pdf-sidebar-background:{};",
            self.pdf_sidebar_background.css()
        ));
        css.push_str(&format!(
            "--pill-background:{};",
            self.pill_background.css()
        ));
        css.push_str(&format!(
            "--pill-border-color:{};",
            self.pill_border_color.css()
        ));
        css.push_str(&format!("--pill-color:{};", self.pill_color.css()));
        css.push_str(&format!(
            "--prompt-background:{};",
            self.prompt_background.css()
        ));
        css.push_str(&format!(
            "--prompt-border-color:{};",
            self.prompt_border_color.css()
        ));
        css.push_str(&format!(
            "--search-clear-button-color:{};",
            self.search_clear_button_color.css()
        ));
        css.push_str(&format!(
            "--search-icon-color:{};",
            self.search_icon_color.css()
        ));
        css.push_str(&format!(
            "--search-result-background:{};",
            self.search_result_background.css()
        ));
        css.push_str(&format!(
            "--setting-item-name-color:{};",
            self.setting_item_name_color.css()
        ));
        css.push_str(&format!(
            "--sidebar-tab-container-background:{};",
            self.sidebar_tab_container_background.css()
        ));
        css.push_str(&format!(
            "--slider-fill-background:{};",
            self.slider_fill_background.css()
        ));
        css.push_str(&format!(
            "--slider-thumb-background:{};",
            self.slider_thumb_background.css()
        ));
        css.push_str(&format!(
            "--slider-track-background:{};",
            self.slider_track_background.css()
        ));
        css.push_str(&format!(
            "--suggestion-background:{};",
            self.suggestion_background.css()
        ));
        css.push_str(&format!(
            "--tab-background-active:{};",
            self.tab_background_active.css()
        ));
        css.push_str(&format!(
            "--tab-container-background:{};",
            self.tab_container_background.css()
        ));
        css.push_str(&format!(
            "--tab-divider-color:{};",
            self.tab_divider_color.css()
        ));
        css.push_str(&format!(
            "--tab-inner-background-hover:{};",
            self.tab_inner_background_hover.css()
        ));
        css.push_str(&format!(
            "--tab-outline-color:{};",
            self.tab_outline_color.css()
        ));
        css.push_str(&format!(
            "--tab-text-color-focused:{};",
            self.tab_text_color_focused.css()
        ));
        css.push_str(&format!(
            "--tab-text-color-focused-active:{};",
            self.tab_text_color_focused_active.css()
        ));
        css.push_str(&format!(
            "--tab-text-color-focused-active-current:{};",
            self.tab_text_color_focused_active_current.css()
        ));
        css.push_str(&format!(
            "--table-border-color:{};",
            self.table_border_color.css()
        ));
        css.push_str(&format!(
            "--table-header-background:{};",
            self.table_header_background.css()
        ));
        css.push_str(&format!(
            "--table-header-border-color:{};",
            self.table_header_border_color.css()
        ));
        css.push_str(&format!(
            "--table-header-color:{};",
            self.table_header_color.css()
        ));
        css.push_str(&format!("--tag-color:{};", self.tag_color.css()));
        css.push_str(&format!("--text-accent:{};", self.text_accent.css()));
        css.push_str(&format!(
            "--text-accent-hover:{};",
            self.text_accent_hover.css()
        ));
        css.push_str(&format!("--text-error:{};", self.text_error.css()));
        css.push_str(&format!("--text-faint:{};", self.text_faint.css()));
        css.push_str(&format!("--text-muted:{};", self.text_muted.css()));
        css.push_str(&format!("--text-normal:{};", self.text_normal.css()));
        css.push_str(&format!("--text-on-accent:{};", self.text_on_accent.css()));
        css.push_str(&format!("--text-selection:{};", self.text_selection.css()));
        css.push_str(&format!("--text-warning:{};", self.text_warning.css()));
        css.push_str(&format!(
            "--toggle-thumb-color:{};",
            self.toggle_thumb_color.css()
        ));
        css.push_str(&format!(
            "--tooltip-background:{};",
            self.tooltip_background.css()
        ));
        css.push_str(&format!("--tooltip-color:{};", self.tooltip_color.css()));
        css.push_str(&format!(
            "--vault-profile-color:{};",
            self.vault_profile_color.css()
        ));
        css.push('}');
        css
    }
}
