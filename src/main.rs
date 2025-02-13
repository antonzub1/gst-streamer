use std::{net::TcpListener, thread, time::Duration};
use actix_web::{web::{self, Data}, App, HttpResponse, HttpServer, Responder};
use color_eyre::Result;
use eyre::eyre;
use gstreamer::{glib::thread_guard::thread_id, message, prelude::*, Message, Structure};
use tokio::{runtime, sync::{mpsc::channel, mpsc::Sender}, time::sleep};
use tracing::{error, info, span, Level};

async fn send_message(tx: Data<Sender<Message>>) -> impl Responder {
    let tid = thread::current().id();
    let sp = span!(Level::INFO, "messaging_thread", tid = ?tid);
    let _messaging_span = sp.enter();
    let structure = Structure::builder("custom-message")
        .field("key", "value")
        .build();
    let message = message::Application::new(structure);
    info!("Sleep for 10 seconds before sending a message");
    sleep(Duration::from_secs(10)).await;
    info!("Wake up after 10 seconds and send message");
    tx.send(message).await.unwrap();
    info!("Sent a message from messaging thread.");
    HttpResponse::Ok().finish()
}


fn main() -> Result<()> {
    color_eyre::install()?;
    gstreamer::init()?;
    tracing_subscriber::fmt().init();


    // let uri = "https://gstreamer.freedesktop.org/data/media/sintel_trailer-480p.webm";
    // let pipeline = gstreamer::parse::launch(&format!("playbin uri={uri}"))
    //     .expect("Failed to parse a pipeline");
    let tid = thread::current().id();
    let mp = span!(Level::INFO, "main_thread", tid = ?tid);
    let _main_span = mp.enter();
    info!("Start streaming.");
    let pipeline = gstreamer::parse::launch("videotestsrc ! autovideosink")?;
    pipeline.set_state(gstreamer::State::Playing)?;
    let bus = pipeline.bus().ok_or_else(|| eyre!("Failed to get a bus"))?;
    bus.add_signal_watch();

    let (tx, mut rx) = channel::<Message>(1);

    let streaming_thread = thread::spawn(move || -> Result<()> {
        let tid = thread_id();
        let sp = span!(Level::INFO, "streaming_thread", tid = %tid);
        let _streaming_span = sp.enter();

        for msg in bus.iter_timed(gstreamer::ClockTime::NONE) {

            use gstreamer::MessageView;

            match msg.view() {
                MessageView::Eos(..) => break,
                MessageView::Error(err) => {
                    error!(
                        "Error from {:?}: {} ({:?})",
                        err.src().map(|s| s.path_string()),
                        err.error(),
                        err.debug()
                    );
                    break;
                },
                MessageView::Application(msg) => {
                    let structure = msg.structure().unwrap();
                    if structure.name() == "custom-message" {
                        info!("Got message from another thread: {:?}", structure);
                    }
                }
                _ => {
                    if let Some(message) = rx.blocking_recv() {
                        bus.post(message)?;
                    }
                },
            }
        }
        pipeline.set_state(gstreamer::State::Null)?;
        Ok(())
    });


    let rt = runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()?;

    let data = Data::new(tx);

    let listener = TcpListener::bind("127.0.0.1:7890")
        .expect("Failed to create TCP listener");
    let app_server = HttpServer::new(move || {
        App::new()
            .route("/message", web::post().to(send_message))
            .app_data(data.clone())
    })
    .listen(listener)?
    .run();

    rt.block_on(app_server)?;
    streaming_thread.join().unwrap()?;
    Ok(())
}
