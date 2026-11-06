// Set the application's icon from the icon theme.
//
// cxx-qt-lib exposes no QIcon binding, so this is the only route. It
// matters beyond aesthetics: on Wayland the compositor resolves a window's
// icon through app_id -> desktop file -> Icon=, but task switchers, the
// about dialog and X11 fallback all read QGuiApplication::windowIcon.
//
// Returns whether the name actually resolved. QIcon::fromTheme yields a
// null icon for a missing name without complaining, which is exactly the
// kind of silent nothing that cost an afternoon on the window bug.

#include <QtGui/QGuiApplication>
#include <QtGui/QIcon>
#include <QtCore/QString>

extern "C" bool bfm_set_window_icon(const char* name)
{
    const QIcon icon = QIcon::fromTheme(QString::fromUtf8(name));
    if (icon.isNull()) {
        return false;
    }
    QGuiApplication::setWindowIcon(icon);
    return true;
}
