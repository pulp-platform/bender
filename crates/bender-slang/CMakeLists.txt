cmake_minimum_required(VERSION 3.24)
project(bender-slang-build LANGUAGES CXX)

include(FetchContent)

FetchContent_Declare(
  slang
  GIT_REPOSITORY https://github.com/MikePopoloski/slang.git
  GIT_TAG        v10.0
  GIT_SHALLOW    TRUE
)
FetchContent_MakeAvailable(slang)

# Expose slang's external headers (e.g. fmt) so the Rust cxx bridge
# can find them. They are not installed by slang's own install rules.
install(DIRECTORY ${slang_SOURCE_DIR}/external/ DESTINATION slang-external)
