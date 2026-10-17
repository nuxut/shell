# Nuxut Qt stili (ADR-0009: C++ yalnız bu eklenti için). Aynı kaynak Qt6 ve Qt5 için derlenir:
#   cmake -S qt-style -B build-qt6 -DQT_MAJOR=6 && cmake --build build-qt6
#   cmake -S qt-style -B build-qt5 -DQT_MAJOR=5 && cmake --build build-qt5
# Kurulum install.sh'tan: <önek>/lib/nuxut/qt<N>/styles/libnuxut-style.so (QT_PLUGIN_PATH ile bulunur).
cmake_minimum_required(VERSION 3.16)
project(nuxut-qt-style LANGUAGES CXX)

set(QT_MAJOR 6 CACHE STRING "Qt ana sürümü (5 ya da 6)")
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
if(NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release)
endif()

if(QT_MAJOR STREQUAL "6")
    find_package(Qt6 REQUIRED COMPONENTS Widgets)
    set(QT_WIDGETS Qt6::Widgets)
elseif(QT_MAJOR STREQUAL "5")
    find_package(Qt5 5.15 REQUIRED COMPONENTS Widgets)
    set(QT_WIDGETS Qt5::Widgets)
else()
    message(FATAL_ERROR "QT_MAJOR 5 ya da 6 olmalı")
endif()

add_library(nuxut-style MODULE src/plugin.cpp src/style.cpp src/tokens.cpp)
target_link_libraries(nuxut-style PRIVATE ${QT_WIDGETS})
target_compile_options(nuxut-style PRIVATE -Wall -Wextra -Werror=return-type)
target_compile_definitions(nuxut-style PRIVATE QT_NO_CAST_FROM_ASCII QT_NO_KEYWORDS)
set_target_properties(nuxut-style PROPERTIES PREFIX "lib" OUTPUT_NAME "nuxut-style"
    LIBRARY_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/plugins/styles")

include(CTest)
if(BUILD_TESTING)
    add_executable(nuxut-style-test tests/style_test.cpp src/tokens.cpp)
    target_include_directories(nuxut-style-test PRIVATE src)
    target_link_libraries(nuxut-style-test PRIVATE ${QT_WIDGETS})
    add_test(NAME nuxut-style-test COMMAND nuxut-style-test)
    set_tests_properties(nuxut-style-test PROPERTIES ENVIRONMENT
        "QT_QPA_PLATFORM=offscreen;QT_PLUGIN_PATH=${CMAKE_BINARY_DIR}/plugins;NUXUT_STYLE_TEST_PLUGIN=${CMAKE_BINARY_DIR}/plugins/styles/libnuxut-style.so")
endif()
