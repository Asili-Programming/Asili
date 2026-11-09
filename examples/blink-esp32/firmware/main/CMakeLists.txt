# No C: `app_main` is the program's `kuu`, in the object `pata jenga --lengo esp32` writes to the
# example's `kilele/`.
idf_component_register()
target_link_libraries(${COMPONENT_LIB} INTERFACE "${CMAKE_CURRENT_SOURCE_DIR}/../../kilele/blink-esp32.o")
