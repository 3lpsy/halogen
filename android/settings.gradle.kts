// Native Android app. Repositories are env-overridable so a Maven proxy can be
// wired in later without code changes (HALOGEN_MAVEN_PROXY, fail-closed when set).
pluginManagement {
    repositories {
        val proxy = System.getenv("HALOGEN_MAVEN_PROXY")
        if (proxy != null) maven(proxy) else {
            google()
            mavenCentral()
            gradlePluginPortal()
        }
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        val proxy = System.getenv("HALOGEN_MAVEN_PROXY")
        if (proxy != null) maven(proxy) else {
            google()
            mavenCentral()
        }
    }
}

rootProject.name = "halogen"
include(":app")
