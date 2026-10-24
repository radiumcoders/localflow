# localflow_llm: lf.cpp plus llama.cpp and ggml, statically, in one shared
# library that exports only the lf_* functions.
cmake_minimum_required(VERSION 3.21)
project(localflow_llm C CXX)

set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
set(CMAKE_C_VISIBILITY_PRESET hidden)
set(CMAKE_CXX_VISIBILITY_PRESET hidden)
set(CMAKE_VISIBILITY_INLINES_HIDDEN ON)
foreach(opt LLAMA_BUILD_COMMON LLAMA_BUILD_TESTS LLAMA_BUILD_TOOLS LLAMA_BUILD_EXAMPLES LLAMA_BUILD_SERVER LLAMA_CURL)
    set(${opt} OFF CACHE BOOL "" FORCE)
endforeach()

add_subdirectory(${LLAMA_DIR} llama EXCLUDE_FROM_ALL)

add_library(localflow_llm SHARED lf.cpp)
target_link_libraries(localflow_llm PRIVATE llama)
target_compile_features(localflow_llm PRIVATE cxx_std_17)
if(CMAKE_SYSTEM_NAME STREQUAL "Linux")
    # Keep the static libraries' symbols (ggml_*) out of the export table.
    target_link_options(localflow_llm PRIVATE -Wl,--exclude-libs,ALL -Wl,--no-undefined)
endif()
install(TARGETS localflow_llm LIBRARY DESTINATION lib RUNTIME DESTINATION bin ARCHIVE DESTINATION lib)
