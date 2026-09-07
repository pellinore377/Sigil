//! Paper 2019/1416, sections 3.1–3.2 and 5.12; experimental Sigil profile.
use crate::{random_bytes, storage::StorageKey, Error, Secret32};
use curve25519_dalek::{
    ristretto::{CompressedRistretto, RistrettoPoint as Point},
    scalar::Scalar,
    traits::Identity,
};
use sha2::{Digest, Sha512};
use std::sync::OnceLock;
use zeroize::{Zeroize, Zeroizing};

const DOMAIN: &[u8] = b"Sigil/private-credentials/v0";
const MAX_CONTEXT: usize = 4096;

struct Parameters {
    w: Point,
    wp: Point,
    x: [Point; 2],
    y: [Point; 3],
    date: Point,
    v: Point,
    a: [Point; 2],
}

fn hash_point(label: &[u8]) -> Point {
    let mut hash = Sha512::new();
    hash.update(DOMAIN);
    hash.update((label.len() as u32).to_be_bytes());
    hash.update(label);
    Point::from_uniform_bytes(&hash.finalize().into())
}

fn parameters() -> &'static Parameters {
    static VALUE: OnceLock<Parameters> = OnceLock::new();
    VALUE.get_or_init(|| Parameters {
        w: hash_point(b"Gw"),
        wp: hash_point(b"Gwprime"),
        x: [hash_point(b"Gx0"), hash_point(b"Gx1")],
        y: [hash_point(b"Gy1"), hash_point(b"Gy2"), hash_point(b"Gy3")],
        date: hash_point(b"Gm3"),
        v: hash_point(b"GV"),
        a: [hash_point(b"Ga1"), hash_point(b"Ga2")],
    })
}

fn random_scalar() -> Result<Scalar, Error> {
    loop {
        let value = Scalar::from_bytes_mod_order_wide(&*random_bytes::<64>()?);
        if value != Scalar::ZERO {
            return Ok(value);
        }
    }
}

fn random_scalars<const N: usize>() -> Result<Zeroizing<[Scalar; N]>, Error> {
    let mut values = Zeroizing::new([Scalar::ZERO; N]);
    for value in values.iter_mut() {
        *value = random_scalar()?;
    }
    Ok(values)
}

fn point(bytes: &[u8]) -> Result<Point, Error> {
    CompressedRistretto(bytes.try_into().map_err(|_| Error::Encoding)?)
        .decompress()
        .ok_or(Error::Encoding)
}

fn nonidentity(value: Point) -> Result<Point, Error> {
    if value == Point::identity() {
        Err(Error::InvalidKey)
    } else {
        Ok(value)
    }
}

pub fn validate_ciphertext(bytes: &[u8]) -> Result<(), Error> {
    if bytes.len() != 64 {
        return Err(Error::Encoding);
    }
    nonidentity(point(&bytes[..32])?)?;
    point(&bytes[32..])?;
    Ok(())
}

fn scalar(bytes: &[u8]) -> Result<Scalar, Error> {
    Option::from(Scalar::from_canonical_bytes(
        bytes.try_into().map_err(|_| Error::Encoding)?,
    ))
    .ok_or(Error::Encoding)
}

fn linear<const N: usize>(bases: &[Point; N], values: &[Scalar; N]) -> Point {
    bases
        .iter()
        .zip(values)
        .fold(Point::identity(), |sum, (g, x)| sum + x * g)
}

struct Proof<const N: usize> {
    challenge: Scalar,
    responses: [Scalar; N],
}

fn challenge<const N: usize, const Q: usize>(
    kind: u8,
    context: &[u8],
    bases: &[[Point; N]; Q],
    targets: &[Point; Q],
    commitments: &[Point; Q],
) -> Result<Scalar, Error> {
    if context.len() > MAX_CONTEXT {
        return Err(Error::Limit);
    }
    let mut hash = Sha512::new();
    hash.update(DOMAIN);
    hash.update([kind, N as u8, Q as u8]);
    hash.update((context.len() as u32).to_be_bytes());
    hash.update(context);
    for ((row, target), commitment) in bases.iter().zip(targets).zip(commitments) {
        for base in row {
            hash.update(base.compress().as_bytes());
        }
        hash.update(target.compress().as_bytes());
        hash.update(commitment.compress().as_bytes());
    }
    Ok(Scalar::from_bytes_mod_order_wide(&hash.finalize().into()))
}

impl<const N: usize> Proof<N> {
    fn create<const Q: usize>(
        kind: u8,
        context: &[u8],
        bases: &[[Point; N]; Q],
        targets: &[Point; Q],
        witness: &[Scalar; N],
    ) -> Result<Self, Error> {
        let nonce = random_scalars::<N>()?;
        let commitments = bases.map(|row| linear(&row, &nonce));
        let challenge = challenge(kind, context, bases, targets, &commitments)?;
        Ok(Self {
            challenge,
            responses: std::array::from_fn(|i| nonce[i] + challenge * witness[i]),
        })
    }

    fn verify<const Q: usize>(
        &self,
        kind: u8,
        context: &[u8],
        bases: &[[Point; N]; Q],
        targets: &[Point; Q],
    ) -> Result<(), Error> {
        let commitments = std::array::from_fn(|i| {
            linear(&bases[i], &self.responses) - self.challenge * targets[i]
        });
        if challenge(kind, context, bases, targets, &commitments)? == self.challenge {
            Ok(())
        } else {
            Err(Error::Authentication)
        }
    }

    fn encode(&self, output: &mut Vec<u8>) {
        output.extend_from_slice(self.challenge.as_bytes());
        for value in self.responses {
            output.extend_from_slice(value.as_bytes());
        }
    }

    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != 32 * (N + 1) {
            return Err(Error::Encoding);
        }
        let mut responses = [Scalar::ZERO; N];
        for (i, response) in responses.iter_mut().enumerate() {
            *response = scalar(&bytes[32 * (i + 1)..32 * (i + 2)])?;
        }
        Ok(Self {
            challenge: scalar(&bytes[..32])?,
            responses,
        })
    }
}

/// Section 5.9 attributes for a 16-byte issuer-scoped UID.
pub struct Attributes([Point; 2]);

impl Attributes {
    pub fn for_uid(uid: &[u8; 16]) -> Result<Self, Error> {
        let mut label = [0; 19];
        label[..3].copy_from_slice(b"UID");
        label[3..].copy_from_slice(uid);
        Ok(Self([nonidentity(hash_point(&label))?, encode_uid(uid)?]))
    }
}

fn encode_uid(uid: &[u8; 16]) -> Result<Point, Error> {
    let mut bytes = Zeroizing::new([0; 32]);
    bytes[1..17].copy_from_slice(uid);
    for counter in 0..=u16::MAX {
        bytes[17..19].copy_from_slice(&counter.to_le_bytes());
        if let Some(value) = CompressedRistretto(*bytes).decompress() {
            if value != Point::identity() {
                return Ok(value);
            }
        }
    }
    Err(Error::Limit)
}

fn decode_uid(value: Point) -> Result<[u8; 16], Error> {
    let bytes = Zeroizing::new(value.compress().to_bytes());
    if bytes[0] != 0 || bytes[19..].iter().any(|b| *b != 0) {
        return Err(Error::Authentication);
    }
    let uid = bytes[1..17].try_into().map_err(|_| Error::Encoding)?;
    if encode_uid(&uid)? != value {
        return Err(Error::Authentication);
    }
    Ok(uid)
}

#[derive(Clone, PartialEq, Eq)]
pub struct IssuerPublic {
    cw: Point,
    i: Point,
}

impl IssuerPublic {
    pub fn to_bytes(&self) -> [u8; 64] {
        let mut out = [0; 64];
        out[..32].copy_from_slice(self.cw.compress().as_bytes());
        out[32..].copy_from_slice(self.i.compress().as_bytes());
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != 64 {
            return Err(Error::Encoding);
        }
        Ok(Self {
            cw: nonidentity(point(&bytes[..32])?)?,
            i: nonidentity(point(&bytes[32..])?)?,
        })
    }
}

/// Separate from transport, identity and group encryption keys.
pub struct Issuer(Zeroizing<[Scalar; 7]>);

struct Mac {
    t: Scalar,
    u: Point,
    v: Point,
}

impl Drop for Mac {
    fn drop(&mut self) {
        self.t.zeroize();
        self.u.zeroize();
        self.v.zeroize();
    }
}

fn issuance_statement(
    public: &IssuerPublic,
    attributes: &Attributes,
    day: u32,
    mac: &Mac,
) -> ([[Point; 7]; 3], [Point; 3]) {
    let p = parameters();
    let o = Point::identity();
    (
        [
            [p.w, p.wp, o, o, o, o, o],
            [o, o, p.x[0], p.x[1], p.y[0], p.y[1], p.y[2]],
            [
                p.w,
                o,
                mac.u,
                mac.t * mac.u,
                attributes.0[0],
                attributes.0[1],
                Scalar::from(day) * p.date,
            ],
        ],
        [public.cw, p.v - public.i, mac.v],
    )
}

impl Issuer {
    pub fn generate() -> Result<Self, Error> {
        Ok(Self(random_scalars()?))
    }

    pub fn seal_checkpoint(&self, key: &StorageKey, binding: &[u8]) -> Result<Vec<u8>, Error> {
        let mut bytes = Zeroizing::new(Vec::with_capacity(232));
        bytes.extend_from_slice(b"SGCI\0\0\0\0");
        for scalar in self.0.iter() {
            bytes.extend_from_slice(scalar.as_bytes());
        }
        key.seal(&bytes, binding)
    }

    pub fn open_checkpoint(
        key: &StorageKey,
        binding: &[u8],
        sealed: &[u8],
        expected: &IssuerPublic,
    ) -> Result<Self, Error> {
        let bytes = key.open(sealed, binding)?;
        if bytes.len() != 232 || &bytes[..8] != b"SGCI\0\0\0\0" {
            return Err(Error::Encoding);
        }
        let mut values = Zeroizing::new([Scalar::ZERO; 7]);
        for (i, value) in values.iter_mut().enumerate() {
            *value = scalar(&bytes[8 + i * 32..8 + (i + 1) * 32])?;
            if *value == Scalar::ZERO {
                return Err(Error::InvalidKey);
            }
        }
        let issuer = Self(values);
        if issuer.public() != *expected {
            return Err(Error::Authentication);
        }
        Ok(issuer)
    }

    pub fn public(&self) -> IssuerPublic {
        let p = parameters();
        let s = &self.0;
        IssuerPublic {
            cw: s[0] * p.w + s[1] * p.wp,
            i: p.v
                - (s[2] * p.x[0] + s[3] * p.x[1] + s[4] * p.y[0] + s[5] * p.y[1] + s[6] * p.y[2]),
        }
    }

    /// Caller authenticates attributes and limits the redemption day before issuance.
    pub fn issue(
        &self,
        attributes: &Attributes,
        day: u32,
        context: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        if context.len() > MAX_CONTEXT {
            return Err(Error::Limit);
        }
        let p = parameters();
        let t = Zeroizing::new(random_scalar()?);
        let u = Point::from_uniform_bytes(&*random_bytes::<64>()?);
        nonidentity(u)?;
        let s = &self.0;
        let mac = Mac {
            t: *t,
            u,
            v: s[0] * p.w
                + (s[2] + s[3] * *t) * u
                + s[4] * attributes.0[0]
                + s[5] * attributes.0[1]
                + s[6] * Scalar::from(day) * p.date,
        };
        let (bases, targets) = issuance_statement(&self.public(), attributes, day, &mac);
        let proof = Proof::create(1, context, &bases, &targets, &self.0)?;
        let mut out = Zeroizing::new(Vec::with_capacity(352));
        out.extend_from_slice(mac.t.as_bytes());
        out.extend_from_slice(mac.u.compress().as_bytes());
        out.extend_from_slice(mac.v.compress().as_bytes());
        proof.encode(&mut out);
        Ok(out)
    }
}

/// Accepted only after checking the issuer's proof against pinned parameters.
pub struct Credential {
    mac: Mac,
    attributes: Attributes,
    day: u32,
    issuer: IssuerPublic,
}

impl Credential {
    pub fn accept(
        public: &IssuerPublic,
        attributes: Attributes,
        day: u32,
        context: &[u8],
        bytes: &[u8],
    ) -> Result<Self, Error> {
        if bytes.len() != 352 {
            return Err(Error::Encoding);
        }
        let mac = Mac {
            t: scalar(&bytes[..32])?,
            u: nonidentity(point(&bytes[32..64])?)?,
            v: point(&bytes[64..96])?,
        };
        let proof = Proof::<7>::decode(&bytes[96..])?;
        let (bases, targets) = issuance_statement(public, &attributes, day, &mac);
        proof.verify(1, context, &bases, &targets)?;
        Ok(Self {
            mac,
            attributes,
            day,
            issuer: public.clone(),
        })
    }

    pub fn present(&self, group: &GroupKey, context: &[u8]) -> Result<Vec<u8>, Error> {
        let binding = presentation_binding(&self.issuer, &group.public(), self.day, context)?;
        let p = parameters();
        let z = Zeroizing::new(random_scalar()?);
        let a = &group.0;
        let ciphertext = group.encrypt(&self.attributes);
        let values = [
            *z * p.x[0] + self.mac.u,
            *z * p.x[1] + self.mac.t * self.mac.u,
            *z * p.y[0] + self.attributes.0[0],
            *z * p.y[1] + self.attributes.0[1],
            *z * p.y[2],
            *z * p.v + self.mac.v,
            ciphertext[0],
            ciphertext[1],
        ];
        let witness = Zeroizing::new([*z, -*z * self.mac.t, -*z * a[0], self.mac.t, a[0], a[1]]);
        let (bases, targets) = presentation_statement(
            &self.issuer,
            group.public_point(),
            &values,
            *z * self.issuer.i,
        );
        let proof = Proof::create(2, &binding, &bases, &targets, &witness)?;
        let mut out = Vec::with_capacity(480);
        for value in values {
            out.extend_from_slice(value.compress().as_bytes());
        }
        proof.encode(&mut out);
        Ok(out)
    }

    pub fn seal_checkpoint(&self, key: &StorageKey, binding: &[u8]) -> Result<Vec<u8>, Error> {
        let mut bytes = Zeroizing::new(Vec::with_capacity(236));
        bytes.extend_from_slice(b"SGCC\0\0\0\0");
        bytes.extend_from_slice(&self.issuer.to_bytes());
        bytes.extend_from_slice(&self.day.to_be_bytes());
        for attribute in self.attributes.0 {
            bytes.extend_from_slice(attribute.compress().as_bytes());
        }
        bytes.extend_from_slice(self.mac.t.as_bytes());
        bytes.extend_from_slice(self.mac.u.compress().as_bytes());
        bytes.extend_from_slice(self.mac.v.compress().as_bytes());
        key.seal(&bytes, binding)
    }

    pub fn open_checkpoint(
        key: &StorageKey,
        binding: &[u8],
        sealed: &[u8],
        expected: &IssuerPublic,
        uid: &[u8; 16],
        day: u32,
    ) -> Result<Self, Error> {
        let bytes = key.open(sealed, binding)?;
        if bytes.len() != 236 || &bytes[..8] != b"SGCC\0\0\0\0" {
            return Err(Error::Encoding);
        }
        let issuer = IssuerPublic::from_bytes(&bytes[8..72])?;
        let stored_day = u32::from_be_bytes(bytes[72..76].try_into().map_err(|_| Error::Encoding)?);
        let attributes = Attributes::for_uid(uid)?;
        if issuer != *expected
            || stored_day != day
            || point(&bytes[76..108])? != attributes.0[0]
            || point(&bytes[108..140])? != attributes.0[1]
        {
            return Err(Error::Authentication);
        }
        Ok(Self {
            issuer,
            attributes,
            day,
            mac: Mac {
                t: scalar(&bytes[140..172])?,
                u: nonidentity(point(&bytes[172..204])?)?,
                v: point(&bytes[204..236])?,
            },
        })
    }
}

/// Section 4.1 key derivation and deterministic UID encryption.
pub struct GroupKey(Zeroizing<[Scalar; 2]>);

impl GroupKey {
    pub fn from_master(master: Secret32) -> Result<Self, Error> {
        let mut values = Zeroizing::new([Scalar::ZERO; 2]);
        for (i, value) in values.iter_mut().enumerate() {
            let mut hash = Sha512::new();
            hash.update(DOMAIN);
            hash.update(b"group-key");
            hash.update([i as u8]);
            hash.update(master.0.as_ref());
            let mut output = hash.finalize();
            *value = Scalar::from_bytes_mod_order_wide(
                output.as_slice().try_into().map_err(|_| Error::State)?,
            );
            output.zeroize();
            if *value == Scalar::ZERO {
                return Err(Error::InvalidKey);
            }
        }
        let key = Self(values);
        nonidentity(key.public_point())?;
        Ok(key)
    }

    fn public_point(&self) -> Point {
        let p = parameters();
        self.0[0] * p.a[0] + self.0[1] * p.a[1]
    }

    pub fn public(&self) -> [u8; 32] {
        self.public_point().compress().to_bytes()
    }

    fn encrypt(&self, attributes: &Attributes) -> [Point; 2] {
        let e1 = self.0[0] * attributes.0[0];
        [e1, self.0[1] * e1 + attributes.0[1]]
    }

    pub fn ciphertext(&self, attributes: &Attributes) -> [u8; 64] {
        let values = self.encrypt(attributes);
        let mut bytes = [0; 64];
        bytes[..32].copy_from_slice(values[0].compress().as_bytes());
        bytes[32..].copy_from_slice(values[1].compress().as_bytes());
        bytes
    }

    pub fn decrypt_uid(&self, bytes: &[u8]) -> Result<[u8; 16], Error> {
        if bytes.len() != 64 {
            return Err(Error::Encoding);
        }
        let first = nonidentity(point(&bytes[..32])?)?;
        let second = point(&bytes[32..])?;
        let uid = Zeroizing::new(decode_uid(second - self.0[1] * first)?);
        let attributes = Attributes::for_uid(&uid)?;
        if self.0[0] * attributes.0[0] != first {
            return Err(Error::Authentication);
        }
        Ok(*uid)
    }
}

fn presentation_binding(
    public: &IssuerPublic,
    group: &[u8; 32],
    day: u32,
    context: &[u8],
) -> Result<[u8; 64], Error> {
    if context.len() > MAX_CONTEXT {
        return Err(Error::Limit);
    }
    let mut hash = Sha512::new();
    hash.update(DOMAIN);
    hash.update(b"presentation-binding");
    hash.update(public.to_bytes());
    hash.update(group);
    hash.update(day.to_be_bytes());
    hash.update((context.len() as u32).to_be_bytes());
    hash.update(context);
    Ok(hash.finalize().into())
}

fn presentation_statement(
    public: &IssuerPublic,
    group: Point,
    c: &[Point; 8],
    z: Point,
) -> ([[Point; 6]; 6], [Point; 6]) {
    let p = parameters();
    let o = Point::identity();
    (
        [
            [public.i, o, o, o, o, o],
            [p.x[1], p.x[0], o, c[0], o, o],
            [o, o, o, o, p.a[0], p.a[1]],
            [p.y[1], o, o, o, o, -c[6]],
            [o, o, p.y[0], o, c[2], o],
            [p.y[2], o, o, o, o, o],
        ],
        [z, c[1], group, c[3] - c[7], c[6], c[4]],
    )
}

impl Issuer {
    /// Returns a proven ciphertext, not an authorization or replay decision.
    pub fn verify_presentation(
        &self,
        group: [u8; 32],
        day: u32,
        context: &[u8],
        bytes: &[u8],
    ) -> Result<[u8; 64], Error> {
        if bytes.len() != 480 {
            return Err(Error::Encoding);
        }
        let public = self.public();
        let binding = presentation_binding(&public, &group, day, context)?;
        let group_point = nonidentity(point(&group)?)?;
        let mut c = [Point::identity(); 8];
        for (i, value) in c.iter_mut().enumerate() {
            *value = point(&bytes[i * 32..(i + 1) * 32])?;
        }
        nonidentity(c[6])?;
        let proof = Proof::<6>::decode(&bytes[256..])?;
        let p = parameters();
        let s = &self.0;
        let z = c[5]
            - (s[0] * p.w
                + s[2] * c[0]
                + s[3] * c[1]
                + s[4] * c[2]
                + s[5] * c[3]
                + s[6] * (c[4] + Scalar::from(day) * p.date));
        let (bases, targets) = presentation_statement(&public, group_point, &c, z);
        proof.verify(2, &binding, &bases, &targets)?;
        bytes[192..256].try_into().map_err(|_| Error::Encoding)
    }
}

#[cfg(test)]
#[path = "private_credential_tests.rs"]
mod tests;
