//! 平台无关的菜单数据。右键菜单 / 托盘菜单由各平台后端渲染成原生菜单。

pub struct MenuEntry {
    /// 命令 ID（0 = 分隔线）。
    pub id: usize,
    pub text: String,
    pub checked: bool,
    /// 子菜单（二级菜单）。
    pub children: Option<Vec<MenuEntry>>,
}

impl MenuEntry {
    pub fn item(id: usize, text: &str) -> MenuEntry {
        MenuEntry { id, text: text.to_string(), checked: false, children: None }
    }

    pub fn check(id: usize, text: &str, checked: bool) -> MenuEntry {
        MenuEntry { id, text: text.to_string(), checked, children: None }
    }

    pub fn submenu(text: &str, children: Vec<MenuEntry>) -> MenuEntry {
        MenuEntry { id: 0, text: text.to_string(), checked: false, children: Some(children) }
    }

    pub fn separator() -> MenuEntry {
        MenuEntry { id: 0, text: String::new(), checked: false, children: None }
    }

    pub fn is_separator(&self) -> bool {
        self.id == 0 && self.children.is_none()
    }
}
