# Export the necessary environment variables to use ESP-IDF.

addIdfEnvVars() {
    # Crude way to detect if $1 is the ESP-IDF derivation.
    if [ -e "$1/tools/idf.py" ]; then
        #export IDF_PATH="$1"
        #export IDF_PYTHON_CHECK_CONSTRAINTS=no
        #export IDF_PYTHON_ENV_PATH="$IDF_PATH/python-env"
        addToSearchPath PATH "$1/tools"
    fi
}

addEnvHooks "$hostOffset" addIdfEnvVars
