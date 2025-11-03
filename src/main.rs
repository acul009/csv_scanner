use iced::application;

use crate::ui::UI;

mod ui;

fn main() {
    application(UI::start, UI::update, UI::view).run().unwrap();
}
