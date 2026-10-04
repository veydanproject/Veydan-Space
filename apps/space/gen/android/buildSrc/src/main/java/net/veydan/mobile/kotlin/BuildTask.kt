import java.io.File
import org.apache.tools.ant.taskdefs.condition.Os
import org.gradle.api.DefaultTask
import org.gradle.api.GradleException
import org.gradle.api.logging.LogLevel
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.TaskAction

// The callback of Gradle into the Tauri CLI (docs/platform-spec.md 13.2, 13.4).
// rootDirRel leads to the crate of the product (apps/<product>), the
// repository root is two levels above it. The CLI is the one of the UI
// project, ui/node_modules/.bin/tauri; it runs from the root like every
// wrapper, pinned to the product by TAURI_APP_PATH and to the UI project by
// TAURI_FRONTEND_PATH: without them it would look for any product.
open class BuildTask : DefaultTask() {
    @Input
    var rootDirRel: String? = null
    @Input
    var target: String? = null
    @Input
    var release: Boolean? = null

    private fun appDir(): File {
        val rootDirRel = rootDirRel ?: throw GradleException("rootDirRel cannot be null")
        return File(project.projectDir, rootDirRel).canonicalFile
    }

    private fun repoRoot(): File = appDir().parentFile.parentFile

    @TaskAction
    fun assemble() {
        val executable = File(repoRoot(), "ui/node_modules/.bin/tauri").path
        try {
            runTauriCli(executable)
        } catch (e: Exception) {
            if (Os.isFamily(Os.FAMILY_WINDOWS)) {
                // Try different Windows-specific extensions
                val fallbacks = listOf(
                    "$executable.exe",
                    "$executable.cmd",
                    "$executable.bat",
                )

                var lastException: Exception = e
                for (fallback in fallbacks) {
                    try {
                        runTauriCli(fallback)
                        return
                    } catch (fallbackException: Exception) {
                        lastException = fallbackException
                    }
                }
                throw lastException
            } else {
                throw e;
            }
        }
    }

    fun runTauriCli(executable: String) {
        val target = target ?: throw GradleException("target cannot be null")
        val release = release ?: throw GradleException("release cannot be null")
        val args = listOf("android", "android-studio-script");

        project.exec {
            workingDir(repoRoot())
            environment("TAURI_APP_PATH", appDir().path)
            environment("TAURI_FRONTEND_PATH", File(repoRoot(), "ui").path)
            executable(executable)
            args(args)
            if (project.logger.isEnabled(LogLevel.DEBUG)) {
                args("-vv")
            } else if (project.logger.isEnabled(LogLevel.INFO)) {
                args("-v")
            }
            if (release) {
                args("--release")
            }
            args(listOf("--target", target))
        }.assertNormalExitValue()
    }
}
