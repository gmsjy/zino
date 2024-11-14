use actix_web::{
    dev::{Payload, ServiceRequest},
    http::{Method, Uri},
    web::Bytes,
    FromRequest, HttpMessage, HttpRequest,
};
use std::{
    borrow::Cow,
    convert::Infallible,
    future,
    net::IpAddr,
    ops::{Deref, DerefMut},
};
use zino_core::{
    error::Error,
    request::{Context, RequestContext},
    state::Data,
};

/// An HTTP request extractor.
pub struct Extractor<T>(T, Payload);

impl<T> Deref for Extractor<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Extractor<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl RequestContext for Extractor<HttpRequest> {
    type Method = Method;
    type Uri = Uri;

    #[inline]
    fn request_method(&self) -> &Self::Method {
        self.method()
    }

    #[inline]
    fn original_uri(&self) -> &Self::Uri {
        self.uri()
    }

    #[inline]
    fn matched_route(&self) -> Cow<'_, str> {
        if let Some(path) = self.match_pattern() {
            path.into()
        } else {
            self.uri().path().into()
        }
    }

    #[inline]
    fn request_path(&self) -> &str {
        self.uri().path()
    }

    #[inline]
    fn get_query_string(&self) -> Option<&str> {
        self.uri().query()
    }

    #[inline]
    fn get_header(&self, name: &str) -> Option<&str> {
        self.headers().get(name)?.to_str().ok()
    }

    #[inline]
    fn client_ip(&self) -> Option<IpAddr> {
        self.connection_info()
            .realip_remote_addr()
            .and_then(|s| s.parse().ok())
    }

    #[inline]
    fn get_context(&self) -> Option<Context> {
        let extensions = self.extensions();
        extensions.get::<Context>().cloned()
    }

    #[inline]
    fn get_data<T: Clone + Send + Sync + 'static>(&self) -> Option<T> {
        self.extensions().get::<Data<T>>().map(|data| data.get())
    }

    #[inline]
    fn set_data<T: Clone + Send + Sync + 'static>(&mut self, value: T) -> Option<T> {
        self.extensions_mut()
            .insert(Data::new(value))
            .map(|data| data.into_inner())
    }

    #[inline]
    async fn read_body_bytes(&mut self) -> Result<Vec<u8>, Error> {
        let bytes = Bytes::from_request(&self.0, &mut self.1)
            .await
            .map_err(Error::from_error)?;
        Ok(bytes.to_vec())
    }
}

impl From<ServiceRequest> for Extractor<HttpRequest> {
    #[inline]
    fn from(request: ServiceRequest) -> Self {
        let (req, payload) = request.into_parts();
        Self(req, payload)
    }
}

impl From<Extractor<HttpRequest>> for ServiceRequest {
    #[inline]
    fn from(extractor: Extractor<HttpRequest>) -> Self {
        ServiceRequest::from_parts(extractor.0, extractor.1)
    }
}

impl From<HttpRequest> for Extractor<HttpRequest> {
    #[inline]
    fn from(request: HttpRequest) -> Self {
        Self(request, Payload::None)
    }
}

impl From<Extractor<HttpRequest>> for HttpRequest {
    #[inline]
    fn from(extractor: Extractor<HttpRequest>) -> Self {
        extractor.0
    }
}

impl FromRequest for Extractor<HttpRequest> {
    type Error = Infallible;
    type Future = future::Ready<Result<Self, Self::Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        future::ready(Ok(Extractor(req.clone(), payload.take())))
    }
}
