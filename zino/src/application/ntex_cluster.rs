use crate::RouterConfigure;
use ntex::{
    rt::System,
    time::{self, Seconds},
    web::{
        self,
        middleware::Compress,
        types::{FormConfig, JsonConfig, PayloadConfig},
        App, HttpServer,
    },
};
use ntex_files::{Files, NamedFile};
use std::path::PathBuf;
use zino_core::{
    application::{Application, Plugin, ServerTag},
    extension::TomlTableExt,
    schedule::AsyncScheduler,
};

/// An HTTP server cluster for `ntex`.
#[derive(Default)]
pub struct NtexCluster {
    /// Custom plugins.
    custom_plugins: Vec<Plugin>,
    /// Default routes.
    default_routes: Vec<RouterConfigure>,
    /// Tagged routes.
    tagged_routes: Vec<(ServerTag, Vec<RouterConfigure>)>,
}

impl Application for NtexCluster {
    type Routes = Vec<RouterConfigure>;

    fn register(mut self, routes: Self::Routes) -> Self {
        self.default_routes = routes;
        self
    }

    fn register_with(mut self, server_tag: ServerTag, routes: Self::Routes) -> Self {
        self.tagged_routes.push((server_tag, routes));
        self
    }

    fn add_plugin(mut self, plugin: Plugin) -> Self {
        self.custom_plugins.push(plugin);
        self
    }

    fn run_with<T: AsyncScheduler + Send + 'static>(self, mut scheduler: T) {
        let app_env = Self::env();
        System::new("prelude").block_on(async {
            Self::load().await;
            super::load_plugins(self.custom_plugins, app_env).await;
        });
        if scheduler.is_ready() {
            System::new("scheduler")
                .system()
                .arbiter()
                .spawn(Box::pin(async move {
                    loop {
                        scheduler.tick().await;

                        // Cannot use `std::thread::sleep` because it blocks the Tokio runtime.
                        time::sleep(scheduler.time_till_next_job()).await;
                    }
                }));
        }

        System::new("main").block_on(async {
            let default_routes = self.default_routes.leak() as &'static [_];
            let tagged_routes = self.tagged_routes.leak() as &'static [_];
            let app_state = Self::shared_state();
            let app_name = Self::name();
            let app_version = Self::version();
            let app_domain = Self::domain();
            let listeners = app_state.listeners();
            let servers = listeners.into_iter().map(|listener| {
                let server_tag = listener.0;
                let addr = listener.1;
                tracing::warn!(
                    server_tag = server_tag.as_str(),
                    app_env = app_env.as_str(),
                    app_name,
                    app_version,
                    "listen on `{addr}`",
                );

                // Server config
                let project_dir = Self::project_dir();
                let default_public_dir = project_dir.join("public");
                let mut public_route_prefix = "/public";
                let mut public_dir = PathBuf::new();
                let mut backlog = 2048; // Maximum number of pending connections
                let mut max_connections = 25000; // Maximum number of concurrent connections
                let mut body_limit = 128 * 1024 * 1024; // 128MB
                let mut request_timeout = 60; // 60 seconds
                if let Some(config) = app_state.get_config("server") {
                    if let Some(dir) = config.get_str("page-dir") {
                        public_route_prefix = "/page";
                        public_dir.push(dir);
                    } else if let Some(dir) = config.get_str("public-dir") {
                        public_dir.push(dir);
                    } else {
                        public_dir = default_public_dir;
                    }
                    if let Some(route_prefix) = config.get_str("public-route-prefix") {
                        public_route_prefix = route_prefix;
                    }
                    if let Some(value) = config.get_i32("backlog") {
                        backlog = value;
                    }
                    if let Some(value) = config.get_usize("max-connections") {
                        max_connections = value;
                    }
                    if let Some(limit) = config.get_usize("body-limit") {
                        body_limit = limit;
                    }
                    if let Some(timeout) = config
                        .get_duration("request-timeout")
                        .and_then(|d| d.as_secs().try_into().ok())
                    {
                        request_timeout = timeout;
                    }
                } else {
                    public_dir = default_public_dir;
                }

                HttpServer::new(move || {
                    let mut app = App::new();
                    if public_dir.exists() {
                        let index_file = public_dir.join("index.html");
                        let favicon_file = public_dir.join("favicon.ico");
                        if index_file.exists() {
                            let index_file_handler = web::get()
                                .to(move || async { NamedFile::open("./public/index.html") });
                            app = app.route("/", index_file_handler);
                        }
                        if favicon_file.exists() {
                            let favicon_file_handler =
                                web::get().to(|| async { NamedFile::open("./public/favicon.ico") });
                            app = app.route("/favicon.ico", favicon_file_handler);
                        }

                        let static_files = Files::new(public_route_prefix, public_dir.clone())
                            .show_files_listing()
                            .index_file("index.html");
                        app = app.service(static_files);
                        tracing::info!(
                            "Static pages `{public_route_prefix}/**` are registered for `{addr}`"
                        );
                    }
                    for route in default_routes {
                        app = app.configure(route);
                    }
                    for (tag, routes) in tagged_routes {
                        if tag == &server_tag || server_tag.is_debug() {
                            for route in routes {
                                app = app.configure(route);
                            }
                        }
                    }

                    app.state(FormConfig::default().limit(body_limit))
                        .state(JsonConfig::default().limit(body_limit))
                        .state(PayloadConfig::default().limit(body_limit))
                        .wrap(Compress::default())
                })
                .stop_runtime()
                .disable_signals()
                .server_hostname(app_domain)
                .backlog(backlog)
                .maxconn(max_connections)
                .client_timeout(Seconds(request_timeout))
                .bind(addr)
                .unwrap_or_else(|err| panic!("fail to create an HTTP server: {err}"))
                .run()
            });
            for result in futures::future::join_all(servers).await {
                if let Err(err) = result {
                    tracing::error!("ntex server error: {err}");
                }
            }
        });
    }
}
