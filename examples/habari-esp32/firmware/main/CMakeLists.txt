# The object and header `pata jenga --lengo esp32` writes to the example's `kilele/`.
set(KILELE "${CMAKE_CURRENT_SOURCE_DIR}/../../kilele")
idf_component_register(SRCS "main.c" INCLUDE_DIRS "${KILELE}")
target_sources(${COMPONENT_LIB} PRIVATE "${KILELE}/habari-esp32.o")
