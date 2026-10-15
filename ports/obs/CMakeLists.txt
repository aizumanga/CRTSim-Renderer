# CC0. Builds CRTSim Renderer's OBS filter against an installed OBS (libobs and its CMake
# package, as obs-studio's development files install them), and installs it with its data:
#
#   crtsim obs-plugin-data -o data          # or the data folder of a release
#   cmake -S ports/obs -B build -DCRTSIM_DATA_DIR=data
#   cmake --build build && cmake --install build --prefix ~/.config/obs-studio/plugins
cmake_minimum_required(VERSION 3.16)
project(crtsim-renderer LANGUAGES C)

find_package(libobs REQUIRED)

add_library(crtsim-renderer MODULE src/crtsim-filter.c)
target_link_libraries(crtsim-renderer PRIVATE OBS::libobs)
set_target_properties(crtsim-renderer PROPERTIES PREFIX "" C_STANDARD 11)
if(MSVC)
  target_compile_options(crtsim-renderer PRIVATE /W4)
else()
  target_compile_options(crtsim-renderer PRIVATE -Wall -Wextra)
endif()

# OBS looks for a user's plugins as <name>/bin/64bit/<name>.so beside <name>/data.
set(CRTSIM_DATA_DIR "" CACHE PATH "The filter's data folder, as crtsim obs-plugin-data writes it")
install(TARGETS crtsim-renderer LIBRARY DESTINATION crtsim-renderer/bin/64bit)
if(CRTSIM_DATA_DIR)
  install(DIRECTORY "${CRTSIM_DATA_DIR}/" DESTINATION crtsim-renderer/data)
endif()

# The headless driver the parity test runs the filter with; Linux, X11 and OpenGL only.
option(CRTSIM_OBS_TEST "Build the filter's headless test driver" OFF)
if(CRTSIM_OBS_TEST)
  find_package(X11 REQUIRED)
  add_executable(crtsim-obs-render test/render.c)
  target_link_libraries(crtsim-obs-render PRIVATE OBS::libobs X11::X11)
endif()
