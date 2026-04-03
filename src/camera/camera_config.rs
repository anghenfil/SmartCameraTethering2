use crate::camera::worker::{CameraConnection, CameraError};
use crate::websocket::scheme::ConfigValue::ButtonPress;
use crate::websocket::scheme::{
    CameraConfig, ConfigSetting, ConfigValue, UpdateCameraConfig, ValidConfigValueRange,
    ValidConfigValues,
};
use gphoto2::widget::{GroupWidget, Widget};
use std::ffi::c_int;

pub async fn get_camera_config(
    connection: &mut CameraConnection,
) -> Result<CameraConfig, CameraError> {
    Ok(connection.camera.config().await?.into())
}

impl From<GroupWidget> for CameraConfig {
    fn from(group_widget: GroupWidget) -> Self {
        let mut res = CameraConfig::default();
        unpack_camera_config(&group_widget, &mut res, "");
        res
    }
}

fn unpack_camera_config(group: &GroupWidget, target: &mut CameraConfig, prefix: &str) {
    let prefix = if prefix.is_empty() {
        group.name()
    } else {
        format!("{}.{}", prefix, group.name())
    };
    for setting in group.children_iter() {
        let setting_name_with_prefix = format!("{}.{}", prefix, setting.name());
        let _id = setting.id();
        let name = setting.name();
        let label = Some(setting.label()).filter(|s| !s.is_empty());
        let info = Some(setting.info()).filter(|s| !s.is_empty());
        let readonly = setting.readonly();

        match setting {
            Widget::Group(group) => unpack_camera_config(&group, target, &prefix),
            Widget::Text(text) => {
                let setting = ConfigSetting {
                    name,
                    label,
                    info,
                    readonly,
                    value: text.value().into(),
                    accepted_values: ValidConfigValues::Text,
                };
                target.values.insert(setting_name_with_prefix, setting);
            }
            Widget::Range(range) => {
                let (valid_range, step) = range.range_and_step();
                let setting = ConfigSetting {
                    name,
                    label,
                    info,
                    readonly,
                    value: range.value().into(),
                    accepted_values: ValidConfigValues::Range(ValidConfigValueRange {
                        min: *valid_range.start(),
                        max: *valid_range.end(),
                        step,
                    }),
                };
                target.values.insert(setting_name_with_prefix, setting);
            }
            Widget::Toggle(toggle) => {
                let setting = ConfigSetting {
                    name,
                    label,
                    info,
                    readonly,
                    value: toggle.toggled().unwrap_or(false).into(),
                    accepted_values: ValidConfigValues::Toggle,
                };
                target.values.insert(setting_name_with_prefix, setting);
            }
            Widget::Radio(radio) => {
                let setting = ConfigSetting {
                    name,
                    label,
                    info,
                    readonly,
                    value: radio.choice().into(),
                    accepted_values: ValidConfigValues::List(radio.choices_iter().collect()),
                };
                target.values.insert(setting_name_with_prefix, setting);
            }
            Widget::Button(_btn) => {
                let setting = ConfigSetting {
                    name,
                    label,
                    info,
                    readonly,
                    value: ButtonPress,
                    accepted_values: ValidConfigValues::ButtonPress,
                };
                target.values.insert(setting_name_with_prefix, setting);
            }
            Widget::Date(date) => {
                let setting = ConfigSetting {
                    name,
                    label,
                    info,
                    readonly,
                    value: (date.timestamp() as i64).into(),
                    accepted_values: ValidConfigValues::Date,
                };
                target.values.insert(setting_name_with_prefix, setting);
            }
        }
    }
}

pub async fn update_camera_config(
    connection: &mut CameraConnection,
    config_update: UpdateCameraConfig,
) -> Result<(), CameraError> {
    for update in config_update.config_updates {
        let name = update.key.split('.').last().unwrap();
        let old_config = connection.camera.config_key::<Widget>(name).await?;
        match &old_config {
            Widget::Group(_) => {
                return Err(CameraError::InvalidConfigValueType);
            }
            Widget::Text(text) => {
                if let ConfigValue::Str(new_value) = update.value {
                    text.set_value(new_value.as_str())?;
                } else {
                    return Err(CameraError::InvalidConfigValueType);
                }
            }
            Widget::Range(range) => {
                if let ConfigValue::F32(new_value) = update.value {
                    range.set_value(new_value);
                } else {
                    return Err(CameraError::InvalidConfigValueType);
                }
            }
            Widget::Toggle(toggle) => {
                if let ConfigValue::Bool(new_value) = update.value {
                    toggle.set_toggled(new_value);
                } else {
                    return Err(CameraError::InvalidConfigValueType);
                }
            }
            Widget::Radio(radio) => {
                if let ConfigValue::Str(new_value) = update.value {
                    radio.set_choice(new_value.as_str())?;
                } else {
                    return Err(CameraError::InvalidConfigValueType);
                }
            }
            Widget::Button(btn) => {
                if let ConfigValue::ButtonPress = update.value {
                    btn.press(&connection.camera)?;
                }
            }
            Widget::Date(camera_date) => {
                if let ConfigValue::Int(new_value) = update.value {
                    camera_date.set_timestamp(new_value as c_int);
                } else {
                    return Err(CameraError::InvalidConfigValueType);
                }
            }
        }
        connection.camera.set_config(&old_config).await?;
    }

    Ok(())
}
