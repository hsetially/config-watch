mod api;
mod app;
mod components;
mod models;
mod storage;
mod ws;

use app::App;

fn main() {
    yew::Renderer::<App>::new().render();
}
