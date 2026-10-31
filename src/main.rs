mod app;
mod app_assets;
mod camera;
mod features;
mod persistence;
#[cfg(target_os = "windows")]
mod single_instance;

fn main() {
    #[cfg(target_os = "windows")]
    let _instance_guard = match single_instance::InstanceGuard::acquire() {
        Ok(Some(guard)) => guard,

        Ok(None) => {
            single_instance::show_message(
                "Vermouth is already running.\n\n\
                 Only one instance of Vermouth can run at a time. \
                 Close the existing instance before launching another.",
                false,
            );
            return;
        }

        Err(error) => {
            single_instance::show_message(
                &format!(
                    "Vermouth could not verify whether another \
                     instance is running.\n\n{error}"
                ),
                true,
            );
            return;
        }
    };

    app::run();
}
