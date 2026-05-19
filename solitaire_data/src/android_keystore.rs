/// Android Keystore token storage via JNI.
///
/// Tokens are serialised to JSON, encrypted with AES-256/GCM/NoPadding using a
/// device-bound key from the Android Keystore, and written atomically to
/// `{data_dir}/ferrous_solitaire/auth_tokens.bin` as `[12-byte IV][ciphertext+GCM-tag]`.
///
/// The file stores a `HashMap<String, TokenBlob>` (keyed by username) so that
/// multiple accounts can coexist without silently overwriting each other.
///
/// The Keystore key survives app restarts but is destroyed on uninstall (or if
/// the user changes biometric/lock credentials, in which case decryption fails
/// and we surface `TokenError::KeychainUnavailable` so the caller knows to
/// prompt re-login — identical semantics to a Linux box without Secret Service).
///
/// Only compiled and linked on `target_os = "android"`.
use jni::{
    objects::{JByteArray, JObject, JObjectArray, JValue, JValueOwned},
    JNIEnv, JavaVM,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::auth_tokens::TokenError;

const KEY_ALIAS: &str = "ferrous_solitaire_token_key";

#[derive(Serialize, Deserialize)]
struct TokenBlob {
    username: String,
    access_token: String,
    refresh_token: String,
}

// ---------------------------------------------------------------------------
// JVM helper
// ---------------------------------------------------------------------------

fn with_jvm<F, R>(f: F) -> Result<R, TokenError>
where
    F: for<'env> FnOnce(&mut JNIEnv<'env>) -> Result<R, jni::errors::Error>,
{
    let app = bevy::android::ANDROID_APP
        .get()
        .ok_or_else(|| TokenError::KeychainUnavailable("ANDROID_APP not initialised".into()))?;

    // SAFETY: vm_as_ptr() is the process-wide JavaVM* set by the Android runtime.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }
        .map_err(|e| TokenError::Keyring(format!("JavaVM: {e}")))?;

    let mut env = vm
        .attach_current_thread_permanently()
        .map_err(|e| TokenError::Keyring(format!("attach: {e}")))?;

    f(&mut env).map_err(|e| TokenError::Keyring(format!("JNI: {e}")))
}

// ---------------------------------------------------------------------------
// Keystore key management
// ---------------------------------------------------------------------------

/// Load the existing AES key from the Android Keystore, or generate one if it
/// doesn't exist yet. Returns a local reference valid for the current JNI frame.
fn load_or_create_key<'local>(env: &mut JNIEnv<'local>) -> jni::errors::Result<JObject<'local>> {
    // KeyStore ks = KeyStore.getInstance("AndroidKeyStore"); ks.load(null);
    let ks_class = env.find_class("java/security/KeyStore")?;
    let ks_type = JValueOwned::from(env.new_string("AndroidKeyStore")?);
    let ks = env
        .call_static_method(
            &ks_class,
            "getInstance",
            "(Ljava/lang/String;)Ljava/security/KeyStore;",
            &[ks_type.borrow()],
        )?
        .l()?;

    let null = JObject::null();
    env.call_method(
        &ks,
        "load",
        "(Ljava/security/KeyStore$LoadStoreParameter;)V",
        &[JValue::Object(&null)],
    )?
    .v()?;

    // Key key = ks.getKey(ALIAS, null)  — char[] password is null for hardware keys
    let alias = JValueOwned::from(env.new_string(KEY_ALIAS)?);
    let null2 = JObject::null();
    let key = env
        .call_method(
            &ks,
            "getKey",
            "(Ljava/lang/String;[C)Ljava/security/Key;",
            &[alias.borrow(), JValue::Object(&null2)],
        )?
        .l()?;

    if !env.is_same_object(&key, JObject::null())? {
        return Ok(key);
    }

    // No key yet — generate AES-256 with GCM block mode.
    let builder_class =
        env.find_class("android/security/keystore/KeyGenParameterSpec$Builder")?;
    let alias2 = JValueOwned::from(env.new_string(KEY_ALIAS)?);
    // PURPOSE_ENCRYPT | PURPOSE_DECRYPT = 1 | 2 = 3
    let purpose = JValueOwned::Int(3);
    let builder = env.new_object(
        &builder_class,
        "(Ljava/lang/String;I)V",
        &[alias2.borrow(), purpose.borrow()],
    )?;

    let str_class = env.find_class("java/lang/String")?;

    // builder.setBlockModes(["GCM"])
    let gcm_str = env.new_string("GCM")?;
    let block_modes: JObjectArray = env.new_object_array(1, &str_class, &gcm_str)?;
    let block_modes_val = JValueOwned::Object(block_modes.into());
    let builder = env
        .call_method(
            &builder,
            "setBlockModes",
            "([Ljava/lang/String;)Landroid/security/keystore/KeyGenParameterSpec$Builder;",
            &[block_modes_val.borrow()],
        )?
        .l()?;

    // builder.setEncryptionPaddings(["NoPadding"])
    let nopad_str = env.new_string("NoPadding")?;
    let enc_pads: JObjectArray = env.new_object_array(1, &str_class, &nopad_str)?;
    let enc_pads_val = JValueOwned::Object(enc_pads.into());
    let builder = env
        .call_method(
            &builder,
            "setEncryptionPaddings",
            "([Ljava/lang/String;)Landroid/security/keystore/KeyGenParameterSpec$Builder;",
            &[enc_pads_val.borrow()],
        )?
        .l()?;

    // KeyGenParameterSpec spec = builder.build()
    let spec = env
        .call_method(
            &builder,
            "build",
            "()Landroid/security/keystore/KeyGenParameterSpec;",
            &[],
        )?
        .l()?;

    // KeyGenerator kg = KeyGenerator.getInstance("AES", "AndroidKeyStore")
    let kg_class = env.find_class("javax/crypto/KeyGenerator")?;
    let aes = JValueOwned::from(env.new_string("AES")?);
    let ks_name = JValueOwned::from(env.new_string("AndroidKeyStore")?);
    let kg = env
        .call_static_method(
            &kg_class,
            "getInstance",
            "(Ljava/lang/String;Ljava/lang/String;)Ljavax/crypto/KeyGenerator;",
            &[aes.borrow(), ks_name.borrow()],
        )?
        .l()?;

    // kg.init(spec); return kg.generateKey()
    let spec_val = JValueOwned::Object(spec);
    env.call_method(
        &kg,
        "init",
        "(Ljava/security/spec/AlgorithmParameterSpec;)V",
        &[spec_val.borrow()],
    )?
    .v()?;

    env.call_method(&kg, "generateKey", "()Ljavax/crypto/SecretKey;", &[])?
        .l()
}

// ---------------------------------------------------------------------------
// AES-GCM encrypt / decrypt
// ---------------------------------------------------------------------------

/// Returns `[12-byte IV][ciphertext+GCM-tag]`.
fn encrypt_gcm(
    env: &mut JNIEnv<'_>,
    key: &JObject<'_>,
    plaintext: &[u8],
) -> jni::errors::Result<Vec<u8>> {
    let cipher_class = env.find_class("javax/crypto/Cipher")?;
    let transform = JValueOwned::from(env.new_string("AES/GCM/NoPadding")?);
    let cipher = env
        .call_static_method(
            &cipher_class,
            "getInstance",
            "(Ljava/lang/String;)Ljavax/crypto/Cipher;",
            &[transform.borrow()],
        )?
        .l()?;

    // cipher.init(Cipher.ENCRYPT_MODE=1, key)
    let mode = JValueOwned::Int(1);
    env.call_method(
        &cipher,
        "init",
        "(ILjava/security/Key;)V",
        &[mode.borrow(), JValue::Object(key)],
    )?
    .v()?;

    // IV is generated by Android's provider; read it back after init.
    let iv_jobj = env.call_method(&cipher, "getIV", "()[B", &[])?.l()?;
    // SAFETY: the method signature guarantees a byte array return.
    let iv_arr = unsafe { JByteArray::from_raw(iv_jobj.into_raw()) };
    let iv = env.convert_byte_array(&iv_arr)?;

    let pt_arr = env.byte_array_from_slice(plaintext)?;
    let pt_val = JValueOwned::Object(pt_arr.into());
    let ct_jobj = env
        .call_method(&cipher, "doFinal", "([B)[B", &[pt_val.borrow()])?
        .l()?;
    // SAFETY: doFinal([B) returns [B.
    let ct_arr = unsafe { JByteArray::from_raw(ct_jobj.into_raw()) };
    let ciphertext = env.convert_byte_array(&ct_arr)?;

    let mut out = Vec::with_capacity(iv.len() + ciphertext.len());
    out.extend_from_slice(&iv);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Expects `data` as `[12-byte IV][ciphertext+GCM-tag]`.
fn decrypt_gcm(
    env: &mut JNIEnv<'_>,
    key: &JObject<'_>,
    data: &[u8],
) -> jni::errors::Result<Vec<u8>> {
    let (iv, ciphertext) = data.split_at(12);

    let cipher_class = env.find_class("javax/crypto/Cipher")?;
    let transform = JValueOwned::from(env.new_string("AES/GCM/NoPadding")?);
    let cipher = env
        .call_static_method(
            &cipher_class,
            "getInstance",
            "(Ljava/lang/String;)Ljavax/crypto/Cipher;",
            &[transform.borrow()],
        )?
        .l()?;

    // GCMParameterSpec spec = new GCMParameterSpec(128, iv)
    let spec_class = env.find_class("javax/crypto/spec/GCMParameterSpec")?;
    let tag_len = JValueOwned::Int(128);
    let iv_arr = env.byte_array_from_slice(iv)?;
    let iv_val = JValueOwned::Object(iv_arr.into());
    let spec = env.new_object(
        &spec_class,
        "(I[B)V",
        &[tag_len.borrow(), iv_val.borrow()],
    )?;

    // cipher.init(Cipher.DECRYPT_MODE=2, key, spec)
    let mode = JValueOwned::Int(2);
    let spec_val = JValueOwned::Object(spec);
    env.call_method(
        &cipher,
        "init",
        "(ILjava/security/Key;Ljava/security/spec/AlgorithmParameterSpec;)V",
        &[mode.borrow(), JValue::Object(key), spec_val.borrow()],
    )?
    .v()?;

    let ct_arr = env.byte_array_from_slice(ciphertext)?;
    let ct_val = JValueOwned::Object(ct_arr.into());
    let pt_jobj = env
        .call_method(&cipher, "doFinal", "([B)[B", &[ct_val.borrow()])?
        .l()?;
    // SAFETY: doFinal([B) returns [B.
    let pt_arr = unsafe { JByteArray::from_raw(pt_jobj.into_raw()) };
    env.convert_byte_array(&pt_arr)
}

// ---------------------------------------------------------------------------
// File helpers
// ---------------------------------------------------------------------------

fn token_file_path() -> Option<PathBuf> {
    crate::platform::data_dir()
        .map(|d| d.join(crate::APP_DIR_NAME).join("auth_tokens.bin"))
}

/// Path where the token file lived before the APP_DIR_NAME subdirectory was
/// introduced. Used only during the one-time migration in `read_map`.
fn legacy_token_file_path() -> Option<PathBuf> {
    crate::platform::data_dir().map(|d| d.join("auth_tokens.bin"))
}

fn read_file_bytes_from(path: &PathBuf) -> Result<Vec<u8>, TokenError> {
    if !path.exists() {
        return Err(TokenError::NotFound(String::new()));
    }
    std::fs::read(path).map_err(|e| TokenError::Keyring(format!("read auth_tokens.bin: {e}")))
}

fn write_file_bytes(data: &[u8]) -> Result<(), TokenError> {
    let path = token_file_path()
        .ok_or_else(|| TokenError::KeychainUnavailable("no data dir".into()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| TokenError::Keyring(format!("create dir: {e}")))?;
    }
    let tmp = path.with_extension("bin.tmp");
    std::fs::write(&tmp, data)
        .map_err(|e| TokenError::Keyring(format!("write auth_tokens.bin.tmp: {e}")))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| TokenError::Keyring(format!("rename auth_tokens: {e}")))
}

/// Decrypt raw bytes from the file and deserialise as `HashMap<String, TokenBlob>`.
///
/// Migration strategy:
/// 1. If the new-path file exists, read and decrypt it.
///    - Try to deserialise as `HashMap<String, TokenBlob>`.
///    - On parse failure (old single-blob format), try `TokenBlob` and convert.
/// 2. If the new-path file does NOT exist but the legacy-path file does, migrate:
///    - Read and decrypt the legacy file.
///    - Deserialise as `TokenBlob` (the only format the legacy path ever used).
///    - Write the result to the new path as a single-entry map.
///    - Delete the legacy file (best-effort; leave it if removal fails).
/// 3. If neither file exists, return an empty map.
fn read_map() -> Result<HashMap<String, TokenBlob>, TokenError> {
    let new_path = token_file_path()
        .ok_or_else(|| TokenError::KeychainUnavailable("no data dir".into()))?;
    let legacy_path = legacy_token_file_path();

    // --- 1. New path exists ---
    if new_path.exists() {
        let data = read_file_bytes_from(&new_path).map_err(|e| match e {
            TokenError::NotFound(_) => TokenError::NotFound(String::new()),
            other => other,
        })?;
        if data.len() < 12 {
            return Err(TokenError::Keyring("auth_tokens.bin corrupt (too short)".into()));
        }
        let plaintext = with_jvm(|env| {
            let key = load_or_create_key(env)?;
            decrypt_gcm(env, &key, &data)
        })?;
        // Try the current multi-user format first.
        if let Ok(map) = serde_json::from_slice::<HashMap<String, TokenBlob>>(&plaintext) {
            return Ok(map);
        }
        // Fall back: old single-blob format written by an earlier binary.
        if let Ok(blob) = serde_json::from_slice::<TokenBlob>(&plaintext) {
            let mut map = HashMap::new();
            map.insert(blob.username.clone(), blob);
            return Ok(map);
        }
        return Err(TokenError::Keyring("auth_tokens.bin unrecognised format".into()));
    }

    // --- 2. Legacy path migration ---
    if let Some(ref lpath) = legacy_path {
        if lpath.exists() {
            let data = read_file_bytes_from(lpath).map_err(|e| match e {
                TokenError::NotFound(_) => TokenError::NotFound(String::new()),
                other => other,
            })?;
            if data.len() >= 12 {
                let plaintext = with_jvm(|env| {
                    let key = load_or_create_key(env)?;
                    decrypt_gcm(env, &key, &data)
                })?;
                if let Ok(blob) = serde_json::from_slice::<TokenBlob>(&plaintext) {
                    let mut map = HashMap::new();
                    map.insert(blob.username.clone(), blob);
                    // Write to the new location, then remove the legacy file.
                    if write_map_inner(&map).is_ok() {
                        let _ = std::fs::remove_file(lpath);
                    }
                    return Ok(map);
                }
            }
            // Legacy file corrupt or unrecognised — treat as empty.
        }
    }

    // --- 3. No file found ---
    Ok(HashMap::new())
}

/// Serialise and encrypt a map, then write it atomically.
fn write_map_inner(map: &HashMap<String, TokenBlob>) -> Result<(), TokenError> {
    let plaintext = serde_json::to_vec(map)
        .map_err(|e| TokenError::Keyring(format!("JSON encode: {e}")))?;
    let encrypted = with_jvm(|env| {
        let key = load_or_create_key(env)?;
        encrypt_gcm(env, &key, &plaintext)
    })?;
    write_file_bytes(&encrypted)
}

// ---------------------------------------------------------------------------
// Public API — mirrors auth_tokens desktop surface exactly.
// ---------------------------------------------------------------------------

/// Encrypt and store `access_token` and `refresh_token` for `username`.
///
/// If tokens already exist for other usernames they are preserved.
/// Any previously stored tokens for `username` are silently replaced.
pub fn store_tokens(
    username: &str,
    access_token: &str,
    refresh_token: &str,
) -> Result<(), TokenError> {
    let mut map = match read_map() {
        Ok(m) => m,
        // If the file is missing or corrupt, start with an empty map so we
        // do not block a fresh login.
        Err(TokenError::NotFound(_)) => HashMap::new(),
        Err(e) => return Err(e),
    };

    map.insert(
        username.to_string(),
        TokenBlob {
            username: username.to_string(),
            access_token: access_token.to_string(),
            refresh_token: refresh_token.to_string(),
        },
    );

    write_map_inner(&map)
}

/// Return the stored access token for `username`.
///
/// Returns [`TokenError::NotFound`] if no token has been stored for this username.
pub fn load_access_token(username: &str) -> Result<String, TokenError> {
    let mut map = read_map()?;
    map.remove(username)
        .map(|b| b.access_token)
        .ok_or_else(|| TokenError::NotFound(username.to_string()))
}

/// Return the stored refresh token for `username`.
///
/// Returns [`TokenError::NotFound`] if no token has been stored for this username.
pub fn load_refresh_token(username: &str) -> Result<String, TokenError> {
    let mut map = read_map()?;
    map.remove(username)
        .map(|b| b.refresh_token)
        .ok_or_else(|| TokenError::NotFound(username.to_string()))
}

/// Delete stored tokens for `username`.
///
/// If other usernames have stored tokens they are left untouched.
/// When this is the last entry in the map the Keystore key is also removed so
/// a future re-login generates a fresh key.
///
/// Missing file or missing Keystore entry are silently ignored.
pub fn delete_tokens(username: &str) -> Result<(), TokenError> {
    let mut map = match read_map() {
        Ok(m) => m,
        Err(TokenError::NotFound(_)) => return Ok(()), // nothing to delete
        Err(e) => return Err(e),
    };

    map.remove(username);

    if map.is_empty() {
        // No more users — remove the file and the Keystore key.
        if let Some(path) = token_file_path() {
            if path.exists() {
                std::fs::remove_file(&path)
                    .map_err(|e| TokenError::Keyring(format!("delete auth_tokens.bin: {e}")))?;
            }
        }

        // Remove the Keystore key so a future re-login generates a fresh key.
        with_jvm(|env| {
            let ks_class = env.find_class("java/security/KeyStore")?;
            let ks_type = JValueOwned::from(env.new_string("AndroidKeyStore")?);
            let ks = env
                .call_static_method(
                    &ks_class,
                    "getInstance",
                    "(Ljava/lang/String;)Ljava/security/KeyStore;",
                    &[ks_type.borrow()],
                )?
                .l()?;

            let null = JObject::null();
            env.call_method(
                &ks,
                "load",
                "(Ljava/security/KeyStore$LoadStoreParameter;)V",
                &[JValue::Object(&null)],
            )?
            .v()?;

            let alias = JValueOwned::from(env.new_string(KEY_ALIAS)?);
            env.call_method(&ks, "deleteEntry", "(Ljava/lang/String;)V", &[alias.borrow()])?
                .v()
        })
    } else {
        // Other users still exist — just rewrite the map without this user.
        write_map_inner(&map)
    }
}
