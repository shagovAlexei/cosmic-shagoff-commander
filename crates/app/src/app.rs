use crate::fl;
use cosmic::app::{Core, Task};
use cosmic::iced::Length;
use cosmic::widget::text;
use cosmic::{Application, Element};

pub const APP_ID: &str = "io.github.shagovAlexei.cosmic-shagoff-commander";

pub struct App {
    core: Core,
}

#[derive(Debug, Clone)]
pub enum Message {}

impl Application for App {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(mut core: Core, _flags: ()) -> (Self, Task<Message>) {
        core.window.header_title = fl!("app-title");
        (Self { core }, Task::none())
    }

    fn view(&self) -> Element<'_, Message> {
        cosmic::widget::container(text::title1(fl!("app-title")))
            .center(Length::Fill)
            .into()
    }
}
