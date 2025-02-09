use std::{net::TcpListener, thread, time::Duration};
use actix_web::{web::{self, Data}, App, HttpResponse, HttpServer, Responder};
use gstreamer::{glib::thread_guard::thread_id, message, prelude::*, Message, Structure};
use tokio::{runtime, sync::{mpsc::channel, mpsc::Sender}};
use tracing::{error, info, span, Level};

async fn send_message(tx: Data<Sender<Message>>) -> impl Responder {
        let tid = thread_id();
        let sp = span!(Level::INFO, "messaging_thread", tid = %tid);
        let _messaging_span = sp.enter();
        let structure = Structure::builder("custom-message")
            .field("key", "value")
            .build();
        let message = message::Application::new(structure);
        info!("Sleep for 10 seconds before sending a message");
        thread::sleep(Duration::from_secs(10));
        info!("Wake up after 10 seconds and send message");
        tx.send(message).await.unwrap();
        info!("Sent a message from messaging thread.");
        HttpResponse::Ok().finish()
}


fn main() {
    gstreamer::init().expect("Failed to initialize GStreamer");
    tracing_subscriber::fmt().init();


    // let uri = "https://gstreamer.freedesktop.org/data/media/sintel_trailer-480p.webm";
    // let pipeline = gstreamer::parse::launch(&format!("playbin uri={uri}"))
    //     .expect("Failed to parse a pipeline");
    let tid = thread_id();
    let mp = span!(Level::INFO, "main_thread", tid = %tid);
    let _main_span = mp.enter();
    info!("Start streaming.");
    let pipeline = gstreamer::parse::launch("videotestsrc ! autovideosink")
        .expect("Failed to parse a pipeline");
    pipeline.set_state(gstreamer::State::Playing)
        .expect("Failed to set the pipeline to the `Playing` state");
    let bus = pipeline.bus().expect("Failed to get the bus for the pipeline");
    bus.add_signal_watch();

    let (tx, mut rx) = channel::<Message>(1);

    let streaming_thread = thread::spawn(move || {
        let tid = thread_id();
        let sp = span!(Level::INFO, "streaming_thread", tid = %tid);
        let _streaming_span = sp.enter();
        if let Some(message) = rx.blocking_recv() {
            bus.post(message).expect("Failed to post a message");
            info!("Got a message from the messaging thread.");
        }

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
                _ => (),
            }
        }
        pipeline.set_state(gstreamer::State::Null)
            .expect("Failed to shutdown a pipeline");
    });


    let rt = runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("Failed to create a runtime");

    let data = Data::new(tx);

    let listener = TcpListener::bind("127.0.0.1:7890")
        .expect("Failed to create TCP listener");
    let app_server = HttpServer::new(move || {
        App::new()
            .route("/message", web::post().to(send_message))
            .app_data(data.clone())
    })
    .listen(listener)
    .expect("Failed to bind a listener")
    .run();

    rt.block_on(app_server).expect("Failed to run a server");
    streaming_thread.join().expect("Failed to join streaming thread");
}
