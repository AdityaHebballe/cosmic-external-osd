use std::sync::LazyLock;
use std::time::Duration;
use std::{env, fs};

#[cfg(target_os = "linux")]
use std::os::unix::fs::FileTypeExt;

use cosmic::app::{Core, Settings, Task};
use cosmic::core::AppType;
use cosmic::iced::event::{self, listen_with};
use cosmic::iced::futures::{
    SinkExt, Stream,
    future::{AbortHandle, Aborted, abortable},
};
use cosmic::iced::platform_specific::shell::commands::corner_radius::corner_radius;
use cosmic::iced::platform_specific::shell::commands::layer_surface::{
    Anchor, KeyboardInteractivity, Layer, destroy_layer_surface,
};
use cosmic::iced::runtime::platform_specific::wayland::CornerRadius;
use cosmic::iced::runtime::platform_specific::wayland::layer_surface::{
    IcedMargin, IcedOutput, SctkLayerSurfaceSettings,
};
use cosmic::iced::window::Id as SurfaceId;
use cosmic::iced::{self, Alignment, Border, Length, Limits, Size, Subscription, stream};
use cosmic::surface::action::{LiveSettings, simple_layer_shell};
use cosmic::{Apply, Element, widget};
use tracing::error;

const APP_ID: &str = "io.github.cosmic_utils.ExternalOsd";
const OBJECT_PATH: &str = "/io/github/cosmic_utils/ExternalOsd";
static OSD_ID: LazyLock<widget::Id> = LazyLock::new(|| widget::Id::new("external-osd"));
#[derive(Clone, Debug)]
enum Msg {
    ShowBrightness(f64),
    ShowAudio(String, String),
    Close,
    Ignore,
    SurfaceSize(SurfaceId, Size),
}

#[derive(Clone)]
struct OsdService {
    output: cosmic::iced::futures::channel::mpsc::Sender<Msg>,
}

#[zbus::interface(name = "io.github.cosmic_utils.ExternalOsd")]
impl OsdService {
    async fn show_brightness(&self, brightness: f64) {
        let mut output = self.output.clone();
        let _ = output
            .send(Msg::ShowBrightness(brightness.clamp(0.0, 1.0)))
            .await;
    }

    async fn show_audio(&self, output_name: String, icon_name: String) {
        let mut output = self.output.clone();
        let _ = output.send(Msg::ShowAudio(output_name, icon_name)).await;
    }
}

fn dbus_subscription() -> impl Stream<Item = Msg> {
    stream::channel(
        8,
        |output: cosmic::iced::futures::channel::mpsc::Sender<Msg>| async move {
            let service = OsdService { output };
            let builder = match zbus::connection::Builder::session() {
                Ok(builder) => builder,
                Err(err) => {
                    error!("cannot connect OSD service to session bus: {err}");
                    return;
                }
            };
            let builder = match builder.name(APP_ID) {
                Ok(builder) => builder,
                Err(err) => {
                    error!("cannot reserve OSD bus name: {err}");
                    return;
                }
            };
            let builder = match builder.serve_at(OBJECT_PATH, service) {
                Ok(builder) => builder,
                Err(err) => {
                    error!("cannot expose OSD service: {err}");
                    return;
                }
            };
            let _connection = match builder.build().await {
                Ok(connection) => connection,
                Err(err) => {
                    error!("cannot start OSD service: {err}");
                    return;
                }
            };
            std::future::pending::<()>().await;
        },
    )
}

#[derive(Clone, Debug)]
enum Content {
    Brightness(f32),
    Audio { name: String, icon: String },
}

impl Content {
    fn width(&self) -> f32 {
        match self {
            Self::Brightness(_) => 392.0,
            Self::Audio { .. } => 240.0,
        }
    }

    fn is_audio(&self) -> bool {
        matches!(self, Self::Audio { .. })
    }
}

struct ActiveOsd {
    id: SurfaceId,
    content: Content,
    timer_abort: AbortHandle,
}

impl ActiveOsd {
    fn timer() -> (Task<Msg>, AbortHandle) {
        let (future, timer_abort) = abortable(async {
            tokio::time::sleep(Duration::from_secs(3)).await;
        });
        (
            cosmic::task::future(async move {
                match future.await {
                    Ok(()) => Msg::Close,
                    Err(Aborted) => Msg::Ignore,
                }
            }),
            timer_abort,
        )
    }

    fn new(content: Content) -> (Self, Task<Msg>) {
        let id = SurfaceId::unique();
        let surface = cosmic::surface::surface_task(simple_layer_shell(
            || LiveSettings {
                corners: Some(CornerRadius::default()),
                ..Default::default()
            },
            move || SctkLayerSurfaceSettings {
                id,
                namespace: "io.github.cosmic_utils.external-osd".into(),
                layer: Layer::Overlay,
                size: None,
                size_limits: Limits::NONE.min_width(1.0).min_height(1.0),
                anchor: Anchor::BOTTOM,
                output: IcedOutput::Active,
                keyboard_interactivity: KeyboardInteractivity::None,
                exclusive_zone: 0,
                margin: IcedMargin {
                    top: 0,
                    right: 0,
                    bottom: 64,
                    left: 0,
                },
                input_zone: Some(Vec::new()),
                ..Default::default()
            },
            None::<fn() -> Element<'static, cosmic::Action<Msg>>>,
        ));
        let (timer, timer_abort) = Self::timer();
        (
            Self {
                id,
                content,
                timer_abort,
            },
            Task::batch([surface, timer]),
        )
    }

    fn update(&mut self, content: Content) -> Task<Msg> {
        self.content = content;
        self.timer_abort.abort();
        let (timer, timer_abort) = Self::timer();
        self.timer_abort = timer_abort;
        timer
    }
}

struct App {
    core: Core,
    active: Option<ActiveOsd>,
}

impl App {
    fn corner_radius_for(&self, height: f32) -> CornerRadius {
        let radius = self.core.system_theme().cosmic().radius_l();
        let limit = (height.max(0.0) / 2.0) as u32;
        CornerRadius {
            top_left: radius[0].min(limit as f32) as u32,
            top_right: radius[1].min(limit as f32) as u32,
            bottom_left: radius[2].min(limit as f32) as u32,
            bottom_right: radius[3].min(limit as f32) as u32,
        }
    }

    fn show(&mut self, content: Content) -> Task<Msg> {
        if let Some(active) = &mut self.active {
            if active.content.is_audio() == content.is_audio() {
                active.update(content)
            } else {
                let old = self.active.take().expect("active OSD exists");
                old.timer_abort.abort();
                let (active, create) = ActiveOsd::new(content);
                self.active = Some(active);
                Task::batch([destroy_layer_surface(old.id), create])
            }
        } else {
            let (active, task) = ActiveOsd::new(content);
            self.active = Some(active);
            task
        }
    }

    fn view_osd(content: &Content) -> Element<'_, Msg> {
        let width = content.width();
        let (icon_name, label, progress) = match content {
            Content::Brightness(value) => (
                "display-brightness-symbolic",
                format!("{}%", (value * 100.0).round() as u32),
                Some(*value),
            ),
            Content::Audio { name, icon } => (icon.as_str(), name.clone(), None),
        };
        let row = if content.is_audio() {
            // Matching left and right icon slots keeps the output name centered.
            cosmic::iced::widget::row![
                widget::container(widget::icon::from_name(icon_name).size(20))
                    .center_x(Length::Fixed(32.0)),
                widget::text::body(label)
                    .width(Length::Fixed(112.0))
                    .center(),
                widget::Space::new().width(Length::Fixed(32.0)),
            ]
            .align_y(Alignment::Center)
            .spacing(8)
        } else {
            // Matches the installed cosmic-osd value OSD layout.
            iced::widget::row![
                widget::container(widget::icon::from_name(icon_name).size(20))
                    .center_x(Length::Fixed(32.0)),
                widget::text::body(label)
                    .width(Length::Fixed(32.0))
                    .center(),
                widget::space::horizontal().width(Length::Fixed(8.0)),
                widget::determinate_linear(progress.expect("brightness has progress"))
                    .girth(4)
                    .width(Length::Fixed(266.0)),
            ]
            .align_y(Alignment::Center)
        };
        let contents = row
            .apply(widget::container)
            .width(Length::Fixed(width))
            .height(Length::Fixed(52.0))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .class(cosmic::theme::Container::custom(move |theme| {
                widget::container::Style {
                    text_color: Some(theme.cosmic().background(theme.transparent).on.into()),
                    background: Some(
                        iced::Color::from(theme.cosmic().background(theme.transparent).base).into(),
                    ),
                    border: Border {
                        radius: theme.cosmic().radius_l().into(),
                        width: 1.0,
                        color: theme.cosmic().bg_divider().into(),
                    },
                    shadow: Default::default(),
                    icon_color: Some(theme.cosmic().background(theme.transparent).on.into()),
                    snap: true,
                }
            }));
        widget::autosize::autosize(
            widget::container(contents)
                .align_x(Alignment::Center)
                .width(Length::Shrink)
                .align_bottom(Length::Shrink),
            OSD_ID.clone(),
        )
        .min_width(1.0)
        .min_height(1.0)
        .into()
    }
}

impl cosmic::Application for App {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Msg;
    const APP_ID: &'static str = APP_ID;

    fn init(mut core: Core, _: ()) -> (Self, Task<Msg>) {
        core.set_app_type(AppType::System);
        (Self { core, active: None }, Task::none())
    }

    fn core(&self) -> &Core {
        &self.core
    }
    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn subscription(&self) -> Subscription<Msg> {
        Subscription::batch([
            Subscription::run_with("external-osd-dbus", |_| dbus_subscription()),
            listen_with(|event, _, id| match event {
                event::Event::Window(iced::window::Event::Opened { size, .. })
                | event::Event::Window(iced::window::Event::Resized(size)) => {
                    Some(Msg::SurfaceSize(id, size))
                }
                _ => None,
            }),
        ])
    }

    fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::ShowBrightness(value) => self.show(Content::Brightness(value as f32)),
            Msg::ShowAudio(name, icon) => self.show(Content::Audio { name, icon }),
            Msg::Close => self
                .active
                .take()
                .map_or_else(Task::none, |active| destroy_layer_surface(active.id)),
            Msg::Ignore => Task::none(),
            Msg::SurfaceSize(id, size) => self
                .active
                .as_ref()
                .filter(|active| active.id == id && size.height >= 2.0)
                .map_or_else(Task::none, |_| {
                    corner_radius(id, Some(self.corner_radius_for(size.height))).discard()
                }),
        }
    }

    fn view(&self) -> Element<'_, Msg> {
        widget::Space::new().into()
    }

    fn view_window(&self, id: SurfaceId) -> Element<'_, Msg> {
        self.active
            .as_ref()
            .filter(|active| active.id == id)
            .map_or_else(
                || widget::Space::new().into(),
                |active| Self::view_osd(&active.content),
            )
    }
}

fn main() -> cosmic::iced::Result {
    ensure_wayland_display();
    cosmic::app::run::<App>(
        Settings::default()
            .no_main_window(true)
            .exit_on_close(false),
        (),
    )
}

fn ensure_wayland_display() {
    if env::var_os("WAYLAND_DISPLAY").is_some() {
        return;
    }
    let Some(runtime_dir) = env::var_os("XDG_RUNTIME_DIR") else {
        return;
    };
    let Ok(entries) = fs::read_dir(runtime_dir) else {
        return;
    };
    let displays: Vec<_> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_socket()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("wayland-"))
        .filter(|name| !name.contains("-render"))
        .collect();
    if let [display] = displays.as_slice() {
        // D-Bus activation does not always inherit the graphical-session
        // environment. With one compositor socket, this is unambiguous.
        unsafe { env::set_var("WAYLAND_DISPLAY", display) };
    }
}
