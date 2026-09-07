/* Test-only independent equations from paper 2019/1416 §§3.2, 5.12.
 * All keys/nonces are fixed synthetic fixtures; never linked into Sigil. */
#include <sodium.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef unsigned char Element[32];
static const unsigned char domain[] = "Sigil/private-credentials/v0";
#define CHECK(x) do { if (!(x)) { fprintf(stderr,"failure: %d\n",__LINE__); exit(1); } } while (0)

static void number(Element x, unsigned n) {
    memset(x, 0, 32);
    for (size_t i = 0; i < 4; i++) x[i] = (unsigned char)(n >> (8*i));
}
static void big32(unsigned char out[4], uint32_t n) {
    for (size_t i = 0; i < 4; i++) out[i] = (unsigned char)(n >> (24-8*i));
}
static void mul(Element out, const Element s, const Element p) {
    if (sodium_is_zero(s,32) || sodium_is_zero(p,32)) memset(out,0,32);
    else CHECK(crypto_scalarmult_ristretto255(out,s,p) == 0);
}
static void add(Element out, const Element p, const Element q) {
    CHECK(crypto_core_ristretto255_add(out,p,q) == 0);
}
static void sub(Element out, const Element p, const Element q) {
    CHECK(crypto_core_ristretto255_sub(out,p,q) == 0);
}
static void hash_start(crypto_hash_sha512_state *state) {
    CHECK(crypto_hash_sha512_init(state) == 0);
    CHECK(crypto_hash_sha512_update(state,domain,sizeof(domain)-1) == 0);
}
static void feed(crypto_hash_sha512_state *state, const void *bytes, size_t n) {
    CHECK(crypto_hash_sha512_update(state,bytes,n) == 0);
}
static void hash_point_bytes(Element out, const void *label, size_t length) {
    crypto_hash_sha512_state state;
    unsigned char size[4], hash[64];
    big32(size,(uint32_t)length);
    hash_start(&state); feed(&state,size,4); feed(&state,label,length);
    CHECK(crypto_hash_sha512_final(&state,hash) == 0);
    CHECK(crypto_core_ristretto255_from_hash(out,hash) == 0);
}
static void hash_point(Element out, const char *label) {
    hash_point_bytes(out,label,strlen(label));
}
static void linear(Element out, const Element *bases, const Element *scalars, size_t n) {
    Element sum = {0}, term;
    for (size_t i = 0; i < n; i++) { mul(term,scalars[i],bases[i]); add(sum,sum,term); }
    memcpy(out,sum,32);
}
static void prove(unsigned char *out, unsigned char kind, size_t n, size_t q,
                  const void *context, size_t context_len, const Element *bases,
                  const Element *targets, const Element *secret, unsigned nonce_start) {
    CHECK(n <= 7 && q <= 6);
    Element nonce[7], commitments[6], product;
    for (size_t i = 0; i < n; i++) number(nonce[i],nonce_start+(unsigned)i);
    for (size_t i = 0; i < q; i++) linear(commitments[i],bases+i*n,nonce,n);
    crypto_hash_sha512_state state;
    unsigned char size[4], head[3] = {kind,(unsigned char)n,(unsigned char)q}, hash[64];
    big32(size,(uint32_t)context_len);
    hash_start(&state); feed(&state,head,3); feed(&state,size,4); feed(&state,context,context_len);
    for (size_t i = 0; i < q; i++) {
        feed(&state,bases+i*n,n*32); feed(&state,targets[i],32); feed(&state,commitments[i],32);
    }
    CHECK(crypto_hash_sha512_final(&state,hash) == 0);
    crypto_core_ristretto255_scalar_reduce(out,hash);
    for (size_t i = 0; i < n; i++) {
        crypto_core_ristretto255_scalar_mul(product,out,secret[i]);
        crypto_core_ristretto255_scalar_add(out+32*(i+1),nonce[i],product);
    }
}
static void emit(const char *name, const unsigned char *bytes, size_t size, int last) {
    CHECK(size <= 512);
    char hex[1025]; sodium_bin2hex(hex,sizeof(hex),bytes,size);
    printf("  \"%s\": \"%s\"%s\n",name,hex,last ? "" : ",");
}

int main(void) {
    CHECK(sodium_init() >= 0);
    Element gw,gwp,gx[2],gy[3],gm,gv,ga[2],attrs[2],sk[7];
    hash_point(gw,"Gw"); hash_point(gwp,"Gwprime");
    hash_point(gx[0],"Gx0"); hash_point(gx[1],"Gx1");
    hash_point(gy[0],"Gy1"); hash_point(gy[1],"Gy2"); hash_point(gy[2],"Gy3");
    hash_point(gm,"Gm3"); hash_point(gv,"GV");
    hash_point(ga[0],"Ga1"); hash_point(ga[1],"Ga2");
    unsigned char uid_label[19] = {'U','I','D'};
    memset(attrs[1],0,32);
    for (unsigned char i = 0; i < 16; i++) { uid_label[3+i] = i; attrs[1][1+i] = i; }
    hash_point_bytes(attrs[0],uid_label,sizeof(uid_label));
    unsigned counter;
    for (counter = 0; counter <= UINT16_MAX; counter++) {
        attrs[1][17] = (unsigned char)counter; attrs[1][18] = (unsigned char)(counter >> 8);
        if (crypto_core_ristretto255_is_valid_point(attrs[1]) && !sodium_is_zero(attrs[1],32)) break;
    }
    CHECK(counter <= UINT16_MAX);
    for (size_t i = 0; i < 7; i++) number(sk[i],1+(unsigned)i);
    Element t,u,v,cw,ip,tmp,day;
    number(t,8); hash_point(u,"fixture-U"); number(day,20000);
    mul(cw,sk[0],gw); mul(tmp,sk[1],gwp); add(cw,cw,tmp);
    Element row[7] = {{0}};
    memcpy(row[2],gx[0],32); memcpy(row[3],gx[1],32);
    for (size_t i = 0; i < 3; i++) memcpy(row[4+i],gy[i],32);
    linear(tmp,row,sk,7); sub(ip,gv,tmp);
    Element ib[3][7] = {{{0}}}, it[3];
    memcpy(ib[0][0],gw,32); memcpy(ib[0][1],gwp,32);
    memcpy(ib[1],row,sizeof(row));
    memcpy(ib[2][0],gw,32); memcpy(ib[2][2],u,32); mul(ib[2][3],t,u);
    memcpy(ib[2][4],attrs[0],32); memcpy(ib[2][5],attrs[1],32); mul(ib[2][6],day,gm);
    linear(v,ib[2],sk,7);
    memcpy(it[0],cw,32); sub(it[1],gv,ip); memcpy(it[2],v,32);
    unsigned char response[352], public[64];
    memcpy(response,t,32); memcpy(response+32,u,32); memcpy(response+64,v,32);
    const char issue_context[] = "issuer.example/key/1";
    prove(response+96,1,7,3,issue_context,sizeof(issue_context)-1,&ib[0][0],it,sk,11);
    memcpy(public,cw,32); memcpy(public+32,ip,32);

    Element a[2],group,z,c[8],witness[6];
    unsigned char master[32]; memset(master,1,32);
    for (unsigned char i = 0; i < 2; i++) {
        crypto_hash_sha512_state state; unsigned char hash[64];
        hash_start(&state); feed(&state,"group-key",9); feed(&state,&i,1); feed(&state,master,32);
        CHECK(crypto_hash_sha512_final(&state,hash) == 0);
        crypto_core_ristretto255_scalar_reduce(a[i],hash);
    }
    linear(group,ga,a,2); number(z,19);
    mul(c[0],z,gx[0]); add(c[0],c[0],u);
    mul(c[1],z,gx[1]); mul(tmp,t,u); add(c[1],c[1],tmp);
    mul(c[2],z,gy[0]); add(c[2],c[2],attrs[0]);
    mul(c[3],z,gy[1]); add(c[3],c[3],attrs[1]);
    mul(c[4],z,gy[2]); mul(c[5],z,gv); add(c[5],c[5],v);
    mul(c[6],a[0],attrs[0]); mul(c[7],a[1],c[6]); add(c[7],c[7],attrs[1]);
    memcpy(witness[0],z,32);
    crypto_core_ristretto255_scalar_mul(tmp,z,t); crypto_core_ristretto255_scalar_negate(witness[1],tmp);
    crypto_core_ristretto255_scalar_mul(tmp,z,a[0]); crypto_core_ristretto255_scalar_negate(witness[2],tmp);
    memcpy(witness[3],t,32); memcpy(witness[4],a[0],32); memcpy(witness[5],a[1],32);
    Element pb[6][6] = {{{0}}}, pt[6], zero = {0};
    memcpy(pb[0][0],ip,32); mul(pt[0],z,ip);
    memcpy(pb[1][0],gx[1],32); memcpy(pb[1][1],gx[0],32); memcpy(pb[1][3],c[0],32); memcpy(pt[1],c[1],32);
    memcpy(pb[2][4],ga[0],32); memcpy(pb[2][5],ga[1],32); memcpy(pt[2],group,32);
    memcpy(pb[3][0],gy[1],32); sub(pb[3][5],zero,c[6]); sub(pt[3],c[3],c[7]);
    memcpy(pb[4][2],gy[0],32); memcpy(pb[4][4],c[2],32); memcpy(pt[4],c[6],32);
    memcpy(pb[5][0],gy[2],32); memcpy(pt[5],c[4],32);
    for (size_t i = 0; i < 6; i++) { linear(tmp,pb[i],witness,6); CHECK(sodium_memcmp(tmp,pt[i],32) == 0); }
    crypto_hash_sha512_state state;
    unsigned char binding[64], size[4], presentation[480];
    const char present_context[] = "group/1/read/nonce/1";
    hash_start(&state); feed(&state,"presentation-binding",20); feed(&state,public,64); feed(&state,group,32);
    big32(size,20000); feed(&state,size,4); big32(size,sizeof(present_context)-1); feed(&state,size,4);
    feed(&state,present_context,sizeof(present_context)-1);
    CHECK(crypto_hash_sha512_final(&state,binding) == 0);
    memcpy(presentation,c,256);
    prove(presentation+256,2,6,6,binding,64,&pb[0][0],pt,witness,31);
    printf("{\n");
    emit("issuer",public,64,0); emit("attributes",(const unsigned char *)attrs,64,0);
    emit("issuance",response,352,0); emit("group",group,32,0); emit("presentation",presentation,480,1);
    printf("}\n");
    return 0;
}
