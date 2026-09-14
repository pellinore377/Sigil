use crate::network::Error;
use http::{Request,Response};
use std::cell::RefCell;
use zeroize::Zeroizing;

type Transport=fn(Request<&[u8]>)->Result<Response<Body>,Error>;
thread_local! {static TRANSPORT:RefCell<Option<Transport>>=const {RefCell::new(None)};}

/// The worker host must enforce redirect refusal, HTTPS and bounded responses.
pub fn install(transport:Transport) {TRANSPORT.with(|slot|*slot.borrow_mut()=Some(transport));}
pub(crate) fn send(request:Request<&[u8]>)->Result<Response<Body>,Error> {
    if request.uri().scheme_str()!=Some("https") {return Err(Error::Configuration);}
    let transport=TRANSPORT.with(|slot|*slot.borrow()).ok_or(Error::Configuration)?;
    transport(request)
}
pub struct Body(Zeroizing<Vec<u8>>);
impl Body {
    pub fn new(bytes:Vec<u8>)->Self {Self(Zeroizing::new(bytes))}
    pub(crate) fn with_config(&mut self)->Reader<'_> {Reader {body:self,limit:0}}
}
pub(crate) struct Reader<'a> {body:&'a mut Body,limit:u64}
impl Reader<'_> {
    pub(crate) fn limit(mut self,limit:u64)->Self {self.limit=limit;self}
    pub(crate) fn read_to_vec(self)->Result<Vec<u8>,Error> {
        if self.body.0.len() as u64>self.limit {return Err(Error::Limit);}
        Ok(std::mem::take(&mut self.body.0))
    }
}
