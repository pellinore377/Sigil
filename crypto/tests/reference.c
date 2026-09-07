/* Independent test-only PQXDH receiver. Never linked into Sigil.
 * Synthetic keys only: Alice=01*32, Bob=02*32, SPK=03*32,
 * ML-KEM seed=04*64, optional OPK=05*32.
 * Build/run instructions and scope are in docs/plan.md.
 */
#include <json-c/json.h>
#include <openssl/core_names.h>
#include <openssl/evp.h>
#include <openssl/kdf.h>
#include <sodium.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CHECK(expr) do { if (!(expr)) { \
    fprintf(stderr, "reference check failed at line %d\n", __LINE__); exit(1); \
} } while (0)

static const char context[] = "Sigil/experimental/pqxdh/v0_CURVE25519_SHA-256_ML-KEM-1024";
static const char initial_context[] = "Sigil/experimental/pqxdh/initial/v0";
static const char triple_initial_context[] = "Sigil/experimental/pqxdh/initial/v1_TripleRatchet";

static size_t field(json_object *object, const char *name, unsigned char *out, size_t cap) {
    json_object *value = NULL;
    CHECK(json_object_object_get_ex(object, name, &value));
    CHECK(json_object_is_type(value, json_type_string));
    const char *hex = json_object_get_string(value);
    size_t length = (size_t)json_object_get_string_len(value), written = 0;
    CHECK(length % 2 == 0 && length / 2 <= cap);
    CHECK(sodium_hex2bin(out, cap, hex, length, NULL, &written, NULL) == 0);
    return written;
}

static EVP_PKEY *curve_key(unsigned char byte) {
    unsigned char secret[32];
    memset(secret, byte, sizeof(secret));
    EVP_PKEY *key = EVP_PKEY_new_raw_private_key_ex(NULL, "X25519", NULL, secret, sizeof(secret));
    CHECK(key != NULL);
    return key;
}

static void check_public(unsigned char byte, const unsigned char *encoded) {
    unsigned char public[32];
    size_t length = sizeof(public);
    EVP_PKEY *key = curve_key(byte);
    CHECK(encoded[0] == 1);
    CHECK(EVP_PKEY_get_raw_public_key(key, public, &length) == 1 && length == 32);
    CHECK(memcmp(public, encoded + 1, 32) == 0);
    EVP_PKEY_free(key);
}

static void dh(unsigned char byte, const unsigned char *public, unsigned char *shared) {
    EVP_PKEY *key = curve_key(byte);
    EVP_PKEY *peer = EVP_PKEY_new_raw_public_key_ex(NULL, "X25519", NULL, public, 32);
    CHECK(peer != NULL);
    EVP_PKEY_CTX *ctx = EVP_PKEY_CTX_new(key, NULL);
    CHECK(ctx != NULL && EVP_PKEY_derive_init(ctx) == 1);
    CHECK(EVP_PKEY_derive_set_peer(ctx, peer) == 1);
    size_t length = 32;
    CHECK(EVP_PKEY_derive(ctx, shared, &length) == 1 && length == 32);
    EVP_PKEY_CTX_free(ctx);
    EVP_PKEY_free(peer);
    EVP_PKEY_free(key);
}

static void hkdf_full(const unsigned char *input, size_t length, const unsigned char salt[32],
                     const void *info, size_t info_len, unsigned char *out, size_t out_len) {
    char digest[] = "SHA256";
    OSSL_PARAM params[] = {
        OSSL_PARAM_construct_utf8_string(OSSL_KDF_PARAM_DIGEST, digest, 0),
        OSSL_PARAM_construct_octet_string(OSSL_KDF_PARAM_KEY, (void *)input, length),
        OSSL_PARAM_construct_octet_string(OSSL_KDF_PARAM_SALT, (void *)salt, 32),
        OSSL_PARAM_construct_octet_string(OSSL_KDF_PARAM_INFO, (void *)info, info_len),
        OSSL_PARAM_construct_end()
    };
    EVP_KDF *algorithm = EVP_KDF_fetch(NULL, "HKDF", NULL);
    CHECK(algorithm != NULL);
    EVP_KDF_CTX *ctx = EVP_KDF_CTX_new(algorithm);
    CHECK(ctx != NULL && EVP_KDF_derive(ctx, out, out_len, params) == 1);
    EVP_KDF_CTX_free(ctx);
    EVP_KDF_free(algorithm);
}

static void hkdf(const unsigned char *input, size_t length, const char *info, unsigned char *out) {
    const unsigned char salt[32] = {0};
    hkdf_full(input, length, salt, info, strlen(info), out, 32);
}

static void kem_secret(const unsigned char *public, const unsigned char *ciphertext, unsigned char *secret) {
    unsigned char seed[64], derived_public[1568];
    memset(seed, 4, sizeof(seed));
    OSSL_PARAM params[] = {
        OSSL_PARAM_construct_octet_string(OSSL_PKEY_PARAM_ML_KEM_SEED, seed, sizeof(seed)),
        OSSL_PARAM_construct_end()
    };
    EVP_PKEY_CTX *generation = EVP_PKEY_CTX_new_from_name(NULL, "ML-KEM-1024", NULL);
    CHECK(generation != NULL && EVP_PKEY_keygen_init(generation) == 1);
    CHECK(EVP_PKEY_CTX_set_params(generation, params) == 1);
    EVP_PKEY *key = NULL;
    CHECK(EVP_PKEY_generate(generation, &key) == 1);
    size_t length = sizeof(derived_public);
    CHECK(EVP_PKEY_get_octet_string_param(key, OSSL_PKEY_PARAM_PUB_KEY,
          derived_public, sizeof(derived_public), &length) == 1);
    CHECK(length == 1568 && memcmp(public, derived_public, length) == 0);
    EVP_PKEY_CTX *ctx = EVP_PKEY_CTX_new(key, NULL);
    CHECK(ctx != NULL && EVP_PKEY_decapsulate_init(ctx, NULL) == 1);
    length = 32;
    CHECK(EVP_PKEY_decapsulate(ctx, secret, &length, ciphertext, 1568) == 1 && length == 32);
    EVP_PKEY_CTX_free(ctx);
    EVP_PKEY_free(key);
    EVP_PKEY_CTX_free(generation);
}

static void verify_signatures(const unsigned char *bundle) {
    unsigned char scalar[32], public[32];
    memset(scalar, 2, sizeof(scalar));
    scalar[0] &= 248;
    scalar[31] &= 127;
    scalar[31] |= 64;
    CHECK(crypto_scalarmult_ed25519_base_noclamp(public, scalar) == 0);
    public[31] &= 127; /* XEdDSA chooses the Edwards point with sign zero. */
    CHECK(crypto_sign_verify_detached(bundle + 74, bundle + 41, 33, public) == 0);
    CHECK(crypto_sign_verify_detached(bundle + 1707, bundle + 138, 1569, public) == 0);
}

static void append(unsigned char *buffer, size_t *length, const void *part, size_t size) {
    CHECK(*length + size <= 4096);
    memcpy(buffer + *length, part, size);
    *length += size;
}

static void check_case(json_object *object) {
    unsigned char bundle[1805], initial[67230], expected[32], plaintext[65536];
    size_t bundle_len = field(object, "bundle", bundle, sizeof(bundle));
    size_t initial_len = field(object, "initial", initial, sizeof(initial));
    CHECK(field(object, "secret", expected, sizeof(expected)) == 32);
    size_t plaintext_len = field(object, "plaintext", plaintext, sizeof(plaintext));
    json_object *ec_value = NULL;
    CHECK(json_object_object_get_ex(object, "ec", &ec_value));
    CHECK(json_object_is_type(ec_value, json_type_boolean));
    int ec = json_object_get_boolean(ec_value);
    CHECK(bundle_len == (size_t)(ec ? 1805 : 1772));
    CHECK(initial_len >= 1694);
    CHECK(memcmp(bundle, "SGPQ\0\1\1\0", 8) == 0);
    CHECK(memcmp(initial, "SGPQ\0\1\2\0", 8) == 0 || memcmp(initial, "SGPQ\0\2\2\0", 8) == 0);
    CHECK(bundle[138] == 2 && bundle[1771] == ec && initial[41] == 1);
    check_public(2, bundle + 8);
    check_public(3, bundle + 41);
    check_public(1, initial + 8);
    if (ec) check_public(5, bundle + 1772);
    verify_signatures(bundle);

    unsigned char transcript[4096], bundle_hash[32];
    size_t length = 0;
    append(transcript, &length, context, strlen(context));
    append(transcript, &length, bundle + 8, 66);
    append(transcript, &length, bundle + 138, 1569);
    append(transcript, &length, bundle + 1771, ec ? 34 : 1);
    CHECK(EVP_Digest(transcript, length, bundle_hash, NULL, EVP_sha256(), NULL) == 1);
    CHECK(memcmp(bundle_hash, initial + 74, 32) == 0);

    unsigned char input[192], secret[32], key[32];
    memset(input, 255, 32);
    dh(3, initial + 9, input + 32);
    dh(2, initial + 42, input + 64);
    dh(3, initial + 42, input + 96);
    if (ec) dh(5, initial + 42, input + 128);
    kem_secret(bundle + 139, initial + 106, input + (ec ? 160 : 128));
    hkdf(input, ec ? 192 : 160, context, secret);
    CHECK(memcmp(secret, expected, 32) == 0);
    hkdf(secret, 32, initial[5] == 2 ? triple_initial_context : initial_context, key);

    length = 0;
    append(transcript, &length, initial + 8, 33);
    append(transcript, &length, bundle + 8, 33);
    append(transcript, &length, bundle + 138, 1569);
    append(transcript, &length, context, strlen(context));
    append(transcript, &length, bundle_hash, 32);
    append(transcript, &length, initial + 41, 33);
    append(transcript, &length, initial + 106, 1568);
    if (initial[5] == 2) append(transcript, &length, triple_initial_context, strlen(triple_initial_context));

    uint32_t claimed = ((uint32_t)initial[1674] << 24) | ((uint32_t)initial[1675] << 16)
        | ((uint32_t)initial[1676] << 8) | initial[1677];
    CHECK(claimed == initial_len - 1678 && claimed - 16 == plaintext_len);
    EVP_CIPHER *cipher = EVP_CIPHER_fetch(NULL, "AES-256-GCM-SIV", NULL);
    EVP_CIPHER_CTX *ctx = EVP_CIPHER_CTX_new();
    unsigned char nonce[12] = {0}, output[65552];
    int produced = 0, final = 0;
    CHECK(cipher != NULL && ctx != NULL);
    CHECK(EVP_DecryptInit_ex2(ctx, cipher, key, nonce, NULL) == 1);
    CHECK(EVP_CIPHER_CTX_ctrl(ctx, EVP_CTRL_AEAD_SET_TAG, 16, initial + initial_len - 16) == 1);
    CHECK(EVP_DecryptUpdate(ctx, NULL, &produced, transcript, (int)length) == 1);
    CHECK(EVP_DecryptUpdate(ctx, output, &produced, initial + 1678, (int)claimed - 16) == 1);
    CHECK(EVP_DecryptFinal_ex(ctx, output + produced, &final) == 1);
    CHECK((size_t)(produced + final) == plaintext_len && memcmp(output, plaintext, plaintext_len) == 0);
    EVP_CIPHER_CTX_free(ctx);
    EVP_CIPHER_free(cipher);
}

static void vector_hex(json_object *vectors, const char *name, const unsigned char *value, size_t length) {
    CHECK(length <= 66000);
    char *hex = malloc(length * 2 + 1);
    CHECK(hex != NULL && sodium_bin2hex(hex, length * 2 + 1, value, length) != NULL);
    json_object_object_add(vectors, name, json_object_new_string(hex));
    free(hex);
}

static void ratchet_vectors(void) {
    /* Independent primitive outputs for the concrete Sigil profile. This is
     * not a second implementation of the Braid/Triple Ratchet state machine. */
    const char braid[] = "Sigil/experimental/braid/v0_MLKEM1024_SHA-256_RaptorQ64";
    const char spqr[] = "Sigil/experimental/spqr/v0_MLKEM1024_SHA-256_RaptorQ64";
    const char triple[] = "Sigil/experimental/triple-ratchet/v0_X25519_MLKEM1024_SHA-256_AES256GCMSIV_RaptorQ64";
    unsigned char zero[32] = {0}, one[32], two[32], three[32], four[32];
    memset(one, 1, 32); memset(two, 2, 32); memset(three, 3, 32); memset(four, 4, 32);
    unsigned char info[4096], output[96];
    size_t n = 0;
    json_object *vectors = json_object_new_object();
    CHECK(vectors != NULL);
    append(info, &n, triple, strlen(triple)); append(info, &n, ":Initialize", 11);
    hkdf_full(one, 32, zero, info, n, output, 64);
    vector_hex(vectors, "split", output, 64);
    hkdf_full(one, 32, two, triple, strlen(triple), output, 32);
    vector_hex(vectors, "hybrid", output, 32);
    n = 0; append(info, &n, spqr, strlen(spqr)); append(info, &n, "Chain Start", 11);
    hkdf_full(one, 32, zero, info, n, output, 96);
    vector_hex(vectors, "spqr_init", output, 96);
    n = 0; append(info, &n, spqr, strlen(spqr)); append(info, &n, "Chain Add Epoch", 15);
    hkdf_full(two, 32, three, info, n, output, 96);
    vector_hex(vectors, "spqr_epoch", output, 96);
    n = 0; append(info, &n, spqr, strlen(spqr)); append(info, &n, "Chain Step", 10);
    const unsigned char counter[4] = {0, 0, 0, 1}; append(info, &n, counter, 4);
    hkdf_full(four, 32, zero, info, n, output, 64);
    vector_hex(vectors, "spqr_step1", output, 64);
    unsigned char next_chain[32]; memcpy(next_chain, output, 32); info[n - 1] = 2;
    hkdf_full(next_chain, 32, zero, info, n, output, 64);
    vector_hex(vectors, "spqr_step2", output, 64);
    const unsigned char epoch[8] = {0, 0, 0, 0, 0, 0, 0, 7};
    n = 0; append(info, &n, braid, strlen(braid)); append(info, &n, ":SCKA Key", 9); append(info, &n, epoch, 8);
    hkdf_full(two, 32, zero, info, n, output, 32);
    vector_hex(vectors, "braid_output", output, 32);
    n = 0; append(info, &n, braid, strlen(braid)); append(info, &n, ":Authenticator Update", 21); append(info, &n, epoch, 8);
    hkdf_full(two, 32, three, info, n, output, 64);
    vector_hex(vectors, "braid_auth", output, 64);
    info[n - 1] = 1;
    hkdf_full(one, 32, zero, info, n, output, 64);
    vector_hex(vectors, "braid_init", output, 64);
    for (int kind = 0; kind < 2; ++kind) {
        n = 0; append(info, &n, braid, strlen(braid));
        const char *label = kind == 0 ? ":ekheader" : ":ciphertext";
        append(info, &n, label, strlen(label)); append(info, &n, epoch, 8);
        if (kind == 0) { memset(info + n, 5, 64); n += 64; }
        else { memset(info + n, 6, 1408); n += 1408; memset(info + n, 7, 160); n += 160; }
        EVP_MAC *algorithm = EVP_MAC_fetch(NULL, "HMAC", NULL);
        CHECK(algorithm != NULL);
        EVP_MAC_CTX *ctx = EVP_MAC_CTX_new(algorithm);
        char digest[] = "SHA256";
        OSSL_PARAM params[] = { OSSL_PARAM_construct_utf8_string(OSSL_MAC_PARAM_DIGEST, digest, 0), OSSL_PARAM_construct_end() };
        size_t written = 0;
        CHECK(ctx != NULL && EVP_MAC_init(ctx, four, 32, params) == 1);
        CHECK(EVP_MAC_update(ctx, info, n) == 1 && EVP_MAC_final(ctx, output, &written, sizeof(output)) == 1 && written == 32);
        vector_hex(vectors, kind == 0 ? "braid_header_mac" : "braid_ciphertext_mac", output, written);
        EVP_MAC_CTX_free(ctx); EVP_MAC_free(algorithm);
    }
    puts(json_object_to_json_string_ext(vectors, JSON_C_TO_STRING_PRETTY));
    json_object_put(vectors);
}

static void xeddsa_vectors(void) {
    json_object *cases = json_object_new_array();
    unsigned orientations = 0;
    for (unsigned n = 1; n <= 32; ++n) {
        unsigned char secret[32], wide[64] = {0}, a[32], public[32], montgomery[32];
        unsigned char random[64], message[24] = "synthetic XEdDSA vector", hash[64];
        unsigned char r[32], h[32], product[32], signature[64], padding[32];
        memset(secret, (int)n, sizeof(secret));
        memcpy(wide, secret, 32); wide[0] &= 248; wide[31] &= 127; wide[31] |= 64;
        crypto_core_ed25519_scalar_reduce(a, wide);
        CHECK(crypto_scalarmult_ed25519_base_noclamp(public, a) == 0);
        unsigned sign = public[31] >> 7;
        orientations |= 1u << sign;
        if (sign) crypto_core_ed25519_scalar_negate(a, a);
        public[31] &= 127;
        CHECK(crypto_scalarmult_curve25519_base(montgomery, secret) == 0);
        for (size_t i = 0; i < sizeof(random); ++i) random[i] = (unsigned char)(n + i);
        message[23] = (unsigned char)n;
        memset(padding, 255, sizeof(padding)); padding[0] = 254;
        EVP_MD_CTX *ctx = EVP_MD_CTX_new(); unsigned written = 0;
        CHECK(ctx != NULL && EVP_DigestInit_ex(ctx, EVP_sha512(), NULL) == 1);
        CHECK(EVP_DigestUpdate(ctx, padding, sizeof(padding)) == 1);
        CHECK(EVP_DigestUpdate(ctx, a, sizeof(a)) == 1);
        CHECK(EVP_DigestUpdate(ctx, message, sizeof(message)) == 1);
        CHECK(EVP_DigestUpdate(ctx, random, sizeof(random)) == 1);
        CHECK(EVP_DigestFinal_ex(ctx, hash, &written) == 1 && written == 64);
        crypto_core_ed25519_scalar_reduce(r, hash);
        CHECK(crypto_scalarmult_ed25519_base_noclamp(signature, r) == 0);
        CHECK(EVP_DigestInit_ex(ctx, EVP_sha512(), NULL) == 1);
        CHECK(EVP_DigestUpdate(ctx, signature, 32) == 1);
        CHECK(EVP_DigestUpdate(ctx, public, sizeof(public)) == 1);
        CHECK(EVP_DigestUpdate(ctx, message, sizeof(message)) == 1);
        CHECK(EVP_DigestFinal_ex(ctx, hash, &written) == 1 && written == 64);
        EVP_MD_CTX_free(ctx);
        crypto_core_ed25519_scalar_reduce(h, hash);
        crypto_core_ed25519_scalar_mul(product, h, a);
        crypto_core_ed25519_scalar_add(signature + 32, r, product);
        CHECK(crypto_sign_verify_detached(signature, message, sizeof(message), public) == 0);
        json_object *value = json_object_new_object();
        vector_hex(value, "secret", secret, sizeof(secret));
        vector_hex(value, "public", montgomery, sizeof(montgomery));
        vector_hex(value, "random", random, sizeof(random));
        vector_hex(value, "message", message, sizeof(message));
        vector_hex(value, "signature", signature, sizeof(signature));
        json_object_object_add(value, "orientation", json_object_new_int((int)sign));
        json_object_array_add(cases, value);
        sodium_memzero(a, sizeof(a)); sodium_memzero(r, sizeof(r));
    }
    CHECK(orientations == 3);
    puts(json_object_to_json_string_ext(cases, JSON_C_TO_STRING_PRETTY));
    json_object_put(cases);
}

static void sender_key_vectors(void) {
    /* Independent AES/HMAC/encoding fixture. Sodium Ed25519 signatures with a
     * positive-orientation public key are valid under the XEdDSA verifier. The
     * separate --xeddsa fixtures test the randomized XEdDSA signing algorithm. */
    unsigned char seed[32], sk[64], pk[32], xsk[32], xpk[32];
    unsigned choice;
    for (choice = 1; choice <= 255; ++choice) {
        memset(seed, (int)choice, sizeof(seed));
        CHECK(crypto_sign_seed_keypair(pk, sk, seed) == 0);
        if ((pk[31] & 128) == 0) break;
    }
    CHECK(choice <= 255);
    CHECK(crypto_sign_ed25519_sk_to_curve25519(xsk, sk) == 0);
    CHECK(crypto_sign_ed25519_pk_to_curve25519(xpk, pk) == 0);
    unsigned char chain[32], distribution[208] = {'S','G','K','D',0,1,0,0};
    memset(chain, 7, sizeof(chain));
    memset(distribution + 8, 1, 32); memset(distribution + 40, 2, 32);
    distribution[79] = 3;
    memset(distribution + 80, 4, 32); memset(distribution + 112, 6, 32);
    memcpy(distribution + 144, chain, 32); memcpy(distribution + 176, xpk, 32);
    json_object *root = json_object_new_object(), *packets = json_object_new_array();
    vector_hex(root, "distribution", distribution, sizeof(distribution));
    vector_hex(root, "signing_secret", xsk, sizeof(xsk));
    json_object_object_add(root, "packets", packets);
    const size_t lengths[] = {0, 19, 257};
    for (unsigned n = 0; n < 3; ++n) {
        unsigned char key[32], next[32], one = 1, two = 2;
        size_t written = 0;
        CHECK(EVP_Q_mac(NULL, "HMAC", NULL, "SHA256", NULL, chain, 32, &one, 1, key, 32, &written) != NULL && written == 32);
        CHECK(EVP_Q_mac(NULL, "HMAC", NULL, "SHA256", NULL, chain, 32, &two, 1, next, 32, &written) != NULL && written == 32);
        unsigned char packet[525] = {'S','G','K','M',0,1,0,0}, plaintext[257];
        memcpy(packet + 8, distribution + 8, 136);
        packet[151] = (unsigned char)n;
        memset(packet + 152, 5 + (int)n, 32);
        const size_t length = lengths[n], cipher_length = length + 16;
        packet[186] = (unsigned char)(cipher_length >> 8);
        packet[187] = (unsigned char)cipher_length;
        for (size_t i = 0; i < length; ++i) plaintext[i] = (unsigned char)(n + i);
        unsigned char nonce[12] = {0};
        EVP_CIPHER *cipher = EVP_CIPHER_fetch(NULL, "AES-256-GCM-SIV", NULL);
        EVP_CIPHER_CTX *ctx = EVP_CIPHER_CTX_new();
        int produced = 0, final = 0;
        CHECK(cipher != NULL && ctx != NULL);
        CHECK(EVP_EncryptInit_ex2(ctx, cipher, key, nonce, NULL) == 1);
        CHECK(EVP_EncryptUpdate(ctx, NULL, &produced, packet, 188) == 1);
        CHECK(EVP_EncryptUpdate(ctx, packet + 188, &produced, plaintext, (int)length) == 1);
        CHECK(EVP_EncryptFinal_ex(ctx, packet + 188 + produced, &final) == 1);
        CHECK((size_t)(produced + final) == length);
        CHECK(EVP_CIPHER_CTX_ctrl(ctx, EVP_CTRL_AEAD_GET_TAG, 16, packet + 188 + length) == 1);
        EVP_CIPHER_CTX_free(ctx); EVP_CIPHER_free(cipher);
        const char domain[] = "Sigil/sender-key-signature/v0";
        unsigned char statement[sizeof(domain) - 1 + 32];
        memcpy(statement, domain, sizeof(domain) - 1);
        unsigned hash_length = 0;
        CHECK(EVP_Digest(packet, 188 + cipher_length, statement + sizeof(domain) - 1, &hash_length, EVP_sha256(), NULL) == 1 && hash_length == 32);
        CHECK(crypto_sign_detached(packet + 188 + cipher_length, NULL, statement, sizeof(statement), sk) == 0);
        CHECK(crypto_sign_verify_detached(packet + 188 + cipher_length, statement, sizeof(statement), pk) == 0);
        json_object *value = json_object_new_object();
        vector_hex(value, "plaintext", plaintext, length);
        vector_hex(value, "packet", packet, 188 + cipher_length + 64);
        json_object_array_add(packets, value);
        memcpy(chain, next, 32);
        sodium_memzero(key, sizeof(key)); sodium_memzero(next, sizeof(next));
    }
    vector_hex(root, "final_chain", chain, sizeof(chain));
    puts(json_object_to_json_string_ext(root, JSON_C_TO_STRING_PRETTY));
    json_object_put(root);
    sodium_memzero(sk, sizeof(sk)); sodium_memzero(xsk, sizeof(xsk));
    sodium_memzero(chain, sizeof(chain));
}

static void big_endian(unsigned char *out, uint64_t value, unsigned width) {
    for (unsigned n = 0; n < width; ++n) out[n] = (unsigned char)(value >> (8 * (width - n - 1)));
}

static void attachment_vectors(void) {
    const uint64_t lengths[] = {0, 19, 1048576 + 17};
    const char key_domain[] = "Sigil/attachment-chunk-key/v0";
    const char list_domain[] = "Sigil/attachment-ciphertext-list/v0";
    json_object *cases = json_object_new_array();
    unsigned char master[32], file[32]; memset(master, 7, 32); memset(file, 6, 32);
    for (unsigned c = 0; c < 3; ++c) {
        uint64_t length = lengths[c]; unsigned count = (unsigned)((length + 1048575) / 1048576);
        if (count == 0) count = 1;
        json_object *item = json_object_new_object(), *parts = json_object_new_array();
        json_object_object_add(item, "length", json_object_new_uint64(length)); json_object_object_add(item, "chunks", parts);
        EVP_MD_CTX *list = EVP_MD_CTX_new(); CHECK(list != NULL);
        unsigned char shape[44]; memcpy(shape, file, 32); big_endian(shape + 32, length, 8); big_endian(shape + 40, count, 4);
        CHECK(EVP_DigestInit_ex(list, EVP_sha256(), NULL) == 1);
        CHECK(EVP_DigestUpdate(list, list_domain, strlen(list_domain)) == 1);
        CHECK(EVP_DigestUpdate(list, shape, sizeof(shape)) == 1);
        for (unsigned index = 0; index < count; ++index) {
            size_t size = (size_t)(length - (uint64_t)index * 1048576); if (size > 1048576) size = 1048576;
            unsigned char *plaintext = malloc(size + 1), *chunk = calloc(size + 84, 1);
            CHECK(plaintext != NULL && chunk != NULL);
            memcpy(chunk, "SGAC\0\x01\0\0", 8); memcpy(chunk + 8, file, 32);
            big_endian(chunk + 40, index, 4); big_endian(chunk + 44, length, 8); big_endian(chunk + 52, size, 4); memset(chunk + 56, (int)index + 8, 12);
            for (size_t i = 0; i < size; ++i) plaintext[i] = (unsigned char)(i * 29 + 17 + index);
            unsigned char info[sizeof(key_domain) - 1 + 12], key[32];
            memcpy(info, key_domain, sizeof(key_domain) - 1); big_endian(info + sizeof(key_domain) - 1, length, 8); big_endian(info + sizeof(key_domain) - 1 + 8, index, 4);
            hkdf_full(master, 32, file, info, sizeof(info), key, 32);
            EVP_CIPHER *cipher = EVP_CIPHER_fetch(NULL, "AES-256-GCM-SIV", NULL); EVP_CIPHER_CTX *ctx = EVP_CIPHER_CTX_new();
            CHECK(cipher != NULL && ctx != NULL); int produced = 0, final = 0;
            CHECK(EVP_EncryptInit_ex2(ctx, cipher, key, chunk + 56, NULL) == 1);
            CHECK(EVP_EncryptUpdate(ctx, NULL, &produced, chunk, 68) == 1);
            CHECK(EVP_EncryptUpdate(ctx, chunk + 68, &produced, plaintext, (int)size) == 1);
            CHECK(EVP_EncryptFinal_ex(ctx, chunk + 68 + produced, &final) == 1 && (size_t)(produced + final) == size);
            CHECK(EVP_CIPHER_CTX_ctrl(ctx, EVP_CTRL_AEAD_GET_TAG, 16, chunk + 68 + size) == 1);
            EVP_CIPHER_CTX_free(ctx); EVP_CIPHER_free(cipher);
            unsigned char hash[32], number[4]; unsigned written = 0;
            CHECK(EVP_Digest(chunk, size + 84, hash, &written, EVP_sha256(), NULL) == 1 && written == 32);
            big_endian(number, index, 4); CHECK(EVP_DigestUpdate(list, number, 4) == 1); CHECK(EVP_DigestUpdate(list, hash, 32) == 1);
            json_object *part = json_object_new_object(); vector_hex(part, "key", key, 32); vector_hex(part, "nonce", chunk + 56, 12); vector_hex(part, "hash", hash, 32);
            if (size < 1024) vector_hex(part, "ciphertext", chunk, size + 84);
            json_object_array_add(parts, part); sodium_memzero(key, 32); sodium_memzero(plaintext, size); free(plaintext); free(chunk);
        }
        unsigned char root[32], descriptor[116] = {'S','G','A','D',0,1,0,0}; unsigned written = 0;
        CHECK(EVP_DigestFinal_ex(list, root, &written) == 1 && written == 32); EVP_MD_CTX_free(list);
        memcpy(descriptor + 8, file, 32); big_endian(descriptor + 40, length, 8); big_endian(descriptor + 48, 1048576, 4); memcpy(descriptor + 52, master, 32); memcpy(descriptor + 84, root, 32);
        vector_hex(item, "root", root, 32); vector_hex(item, "descriptor", descriptor, sizeof(descriptor)); json_object_array_add(cases, item);
    }
    puts(json_object_to_json_string_ext(cases, JSON_C_TO_STRING_PRETTY)); json_object_put(cases); sodium_memzero(master, 32);
}

int main(int argc, char **argv) {
    CHECK(argc == 2 && sodium_init() >= 0);
    if (strcmp(argv[1], "--ratchet") == 0) { ratchet_vectors(); return 0; }
    if (strcmp(argv[1], "--xeddsa") == 0) { xeddsa_vectors(); return 0; }
    if (strcmp(argv[1], "--sender-keys") == 0) { sender_key_vectors(); return 0; }
    if (strcmp(argv[1], "--attachments") == 0) { attachment_vectors(); return 0; }
    json_object *cases = json_object_from_file(argv[1]);
    CHECK(cases != NULL && json_object_is_type(cases, json_type_array));
    CHECK(json_object_array_length(cases) == 2);
    for (size_t i = 0; i < 2; ++i) check_case(json_object_array_get_idx(cases, i));
    json_object_put(cases);
    puts("Both PQXDH fixtures independently verified (OpenSSL + libsodium).");
    return 0;
}
