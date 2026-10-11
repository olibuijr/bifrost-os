plugins {
    id("com.android.application")
}

android {
    namespace = "is.olibuijr.midget"
    compileSdk = 36

    defaultConfig {
        applicationId = "is.olibuijr.midget"
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
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
