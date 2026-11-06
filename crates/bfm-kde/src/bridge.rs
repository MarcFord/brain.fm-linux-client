//! Rust side of the Qt bridge.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, probe)]
        type Session = super::SessionRust;
    }
}

pub struct SessionRust {
    probe: cxx_qt_lib::QString,
}

impl Default for SessionRust {
    fn default() -> Self {
        Self { probe: cxx_qt_lib::QString::from("bridge alive") }
    }
}
