#[cfg(target_os = "android")]
mod android;

#[cfg(not(target_os = "android"))]
mod stub;

#[cfg(not(target_os = "android"))]
pub use stub::GpgsClient;

#[cfg(target_os = "android")]
pub use android::GpgsClient;
