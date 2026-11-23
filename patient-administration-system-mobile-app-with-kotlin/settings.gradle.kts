// The patient mobile app for Android (spec/mobile-apps/index.md).
//
// Lily Design System for Kotlin is not on Maven Central yet, so it is an included build from a local checkout:
// -PlilyKotlinPath=..., or LILY_KOTLIN_PATH, or the default layout beside this repository's parent folder.
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}
rootProject.name = "patient-administration-system-mobile-app-with-kotlin"

val lilyKotlinPath: String = providers.gradleProperty("lilyKotlinPath")
    .orElse(providers.environmentVariable("LILY_KOTLIN_PATH"))
    .getOrElse("../../../lilydesignsystem/lily-design-system/lily-design-system-kotlin")

includeBuild(lilyKotlinPath) {
    dependencySubstitution {
        substitute(module("io.github.lilydesignsystem:lily-compose")).using(project(":lily-compose"))
        substitute(module("io.github.lilydesignsystem:lily-compose-themes")).using(project(":lily-compose-themes"))
    }
}

include(":core", ":app")
