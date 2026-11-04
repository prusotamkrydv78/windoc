plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.notchy.app"
    compileSdk = 36

    defaultConfig {
        applicationId = "com.notchy.app"
        minSdk = 28
        targetSdk = 36
        versionCode = 4
        versionName = "0.4.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    testOptions {
        unitTests.isIncludeAndroidResources = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin {
    jvmToolchain(17)
}

// Plain Android views and platform APIs; the one library is Google's code scanner, which
// scans the PC's pairing QR without a camera permission (Play services does the scanning).
dependencies {
    implementation("com.google.android.gms:play-services-code-scanner:16.1.0")
    // Renders the island to PNGs on the PC (no phone or emulator needed) to check the design.
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.robolectric:robolectric:4.14.1")
}
