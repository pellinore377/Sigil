//! A deterministic RNG for vectors and tests only: a BLAKE3 XOF stream.
//! Never use it in production. Implements both RNG trait generations in use
//! by the dependencies (rand_core 0.6 for ML-KEM, 0.10 for blind RSA).

pub struct TestRng(blake3::OutputReader);

impl TestRng {
    pub fn new(seed: &[u8]) -> Self {
        TestRng(
            blake3::Hasher::new_derive_key("sigil v1 test rng")
                .update(seed)
                .finalize_xof(),
        )
    }
    fn u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        self.0.fill(&mut b);
        u32::from_le_bytes(b)
    }
    fn u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        self.0.fill(&mut b);
        u64::from_le_bytes(b)
    }
}

impl rand_core::RngCore for TestRng {
    fn next_u32(&mut self) -> u32 {
        self.u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.u64()
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.0.fill(dest)
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.0.fill(dest);
        Ok(())
    }
}
impl rand_core::CryptoRng for TestRng {}

impl rand_core10::TryRng for TestRng {
    type Error = core::convert::Infallible;
    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(self.u32())
    }
    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(self.u64())
    }
    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        self.0.fill(dst);
        Ok(())
    }
}
impl rand_core10::TryCryptoRng for TestRng {}
