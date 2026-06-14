// SPDX-License-Identifier: GPL-3.0-only

use cosmic::{
    Element,
    app::Core,
    theme,
    widget::{
        self,
        menu::{self, key_bind::KeyBind},
    },
};
use std::collections::HashMap;

use crate::{Action, Config, Message, fl};

pub fn menu_bar<'a>(
    _core: &Core,
    _config: &Config,
    key_binds: &HashMap<KeyBind, Action>,
) -> Element<'a, Message> {
    menu::bar(vec![menu::Tree::with_children(
        widget::RcElementWrapper::new(
            widget::button::icon(widget::icon::from_name("open-menu-symbolic"))
                .padding([4, 12])
                .class(theme::Button::MenuRoot)
                .into(),
        ),
        menu::items(
            key_binds,
            vec![
                menu::Item::Button(fl!("menu-settings"), None, Action::Settings),
                menu::Item::Divider,
                menu::Item::Button(fl!("menu-about"), None, Action::About),
            ],
        ),
    )])
    .item_height(menu::ItemHeight::Dynamic(40))
    .item_width(menu::ItemWidth::Uniform(320))
    .spacing(theme::active().cosmic().spacing.space_xxxs.into())
    .into()
}
