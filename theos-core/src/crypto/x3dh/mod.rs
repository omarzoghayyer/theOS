// crypto/x3dh/ — X3DH session establishment (Signal protocol).
// Produces the initial shared root key + peer ratchet public key that seed the
// Double Ratchet (crypto/ratchet/). Built: prekey. Pending: handshake.
pub mod prekey;
pub mod handshake;
