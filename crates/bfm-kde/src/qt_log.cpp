// Route Qt's own diagnostics to stderr.
//
// This exists because of a concrete failure: `QQmlApplicationEngine::load`
// on a deliberately nonexistent URL produced no output at all. A working
// engine always complains about a missing component, so the silence was
// itself the bug worth chasing — and with nothing on stderr, every other
// question about why no window appears is guesswork.
//
// The handler writes with fprintf rather than qInstallMessageHandler's
// default, so output cannot be lost to whatever is swallowing it, and
// flushes immediately so a message is not stranded in a buffer when the
// process is killed.

#include <QtCore/QLoggingCategory>
#include <QtCore/QString>
#include <QtCore/QtGlobal>

#include <cstdio>

namespace {

const char* levelName(QtMsgType type)
{
    switch (type) {
    case QtDebugMsg:    return "debug";
    case QtInfoMsg:     return "info";
    case QtWarningMsg:  return "warning";
    case QtCriticalMsg: return "critical";
    case QtFatalMsg:    return "fatal";
    }
    return "unknown";
}

void handler(QtMsgType type, const QMessageLogContext& context, const QString& message)
{
    const QByteArray text = message.toLocal8Bit();

    // The category and source location are what make a QML error
    // actionable; the default pattern often omits them.
    const char* category = context.category ? context.category : "default";
    const char* file = context.file ? context.file : "";
    const int line = context.line;

    if (file[0] != '\0') {
        std::fprintf(stderr, "[qt %s/%s] %s (%s:%d)\n",
                     levelName(type), category, text.constData(), file, line);
    } else {
        std::fprintf(stderr, "[qt %s/%s] %s\n",
                     levelName(type), category, text.constData());
    }
    std::fflush(stderr);
}

} // namespace

extern "C" void bfm_install_qt_message_handler()
{
    // Warnings and above are on by default, but debug and info are not, and
    // QML import tracing lives there. Turn everything on: this binary is
    // only noisy when BFM_QT_DEBUG asks it to be.
    if (qEnvironmentVariableIsSet("BFM_QT_DEBUG")) {
        QLoggingCategory::setFilterRules(QStringLiteral("*=true\nqt.*.debug=true"));
    }
    qInstallMessageHandler(handler);
}
