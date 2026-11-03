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
        versionCode = 2
        versionName = "0.2.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
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
}
